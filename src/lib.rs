//! Momawd's Optimization Mod (`momawds_optimization`): sandboxed WebAssembly all-in-one FPS & TPS
//! optimizer for Horizon 26.3.
//!
//! Combines client frametime optimizations (dynamic unfocused/minimized/idle FPS governor,
//! distance-based item-frame 3D content culling, crowd name-tag and ambient effect swirl relief)
//! with lossless world/server TPS optimizations (exact `u128` experience orb clumping, instant
//! multi-orb pickup, adaptive MSPT wildlife tick pacing, and chain-explosion entity blast guards),
//! plus configurable presets and an optional corner mini-HUD (`FPS / 1% Low / MSPT`).

pub mod frame;
pub mod hud;
pub mod settings;
pub mod tps;
pub mod xp;

use std::sync::Mutex;

use mod_api::runtime::Flow;
use mod_api::{
    self, ClientHost, EffectParticleModificationEvent, EntityTickPreEvent,
    ExperienceOrbCanMergeEvent, ExperienceOrbMergeEvent, ExplosionDetonateEvent,
    FramerateLimitEvent, Hooks, Host, PickupXpEvent, Registrar, RegistrationContext,
    RenderFramePreEvent, RenderGuiPostEvent, RenderItemInFrameEvent, RenderNameTagEvent,
    ServerHost, ServerTickEvent, ServerTickPhase,
};

use crate::frame::{FrameTracker, block_dist_sq, compute_fps_cap, should_suppress_crowd_nametag};
use crate::hud::{BOX_PADDING, HUD_BACKGROUND, box_size, format_hud_text, hud_ink_color};
use crate::settings::{ActiveConfig, HudStyle, Preset, settings_specs};
use crate::tps::{explosion_entity_cap, should_pace_entity_tick};
use crate::xp::{OrbMergePlan, plan_orb_merge, resolve_orb_pickup};

#[derive(Default)]
struct RuntimeState {
    config: ActiveConfig,
    client_seen: bool,
    frames: FrameTracker,
    camera_pos: Option<[f64; 3]>,
    server_tick_index: u64,
    server_mspt: Option<f64>,
    server_has_time: bool,
    explosions_this_tick: u32,
    saved_actions: u64,
}

#[derive(Default)]
pub struct MomawdsOptimization {
    // Never held across a host call: Horizon host calls can re-enter guest hooks.
    state: Mutex<RuntimeState>,
}

impl MomawdsOptimization {
    fn lock(&self) -> std::sync::MutexGuard<'_, RuntimeState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[mod_api::horizon_mod]
impl Hooks for MomawdsOptimization {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        for spec in settings_specs() {
            let _ = registrar.register_setting(spec);
        }
    }

    /// Dynamic FPS limiter for unfocused, minimized, and idle menu windows.
    fn framerate_limit(&self, host: &dyn ClientHost, event: &mut FramerateLimitEvent) {
        let cfg = ActiveConfig::from_client(host);
        {
            let mut st = self.lock();
            st.config = cfg;
            st.client_seen = true;
        }
        event.limit = compute_fps_cap(
            event.limit,
            event.focused,
            event.minimized,
            event.in_world,
            &cfg,
        );
    }

    /// Updates rolling 5-second FPS / 1% low counter and caches per-frame camera + settings so
    /// per-entity rendering hooks run with zero WASM host-import overhead.
    fn render_frame_pre(&self, host: &mut dyn ClientHost, event: &mut RenderFramePreEvent) {
        let cfg = ActiveConfig::from_client(host);
        let cam_pos = host.camera().map(|cam| cam.position);
        let elapsed = event.elapsed.as_secs_f64();

        let mut st = self.lock();
        st.config = cfg;
        st.client_seen = true;
        st.camera_pos = cam_pos;
        st.frames.record_frame(elapsed);
        if cam_pos.is_none() {
            // Clear integrated server MSPT readout once the player leaves a local world.
            st.server_mspt = None;
        }
    }

    /// Distance-culls expensive 3D item/map renders inside distant item frames while keeping the
    /// frame itself visible.
    fn render_item_in_frame(
        &self,
        _: &dyn ClientHost,
        event: &mut RenderItemInFrameEvent<'_>,
    ) -> Flow {
        let mut st = self.lock();
        if !st.config.enabled || !st.config.item_frame_cull {
            return Flow::Continue;
        }
        let Some(cam_pos) = st.camera_pos else {
            return Flow::Continue;
        };
        let under_pressure = st.frames.reading().under_pressure();
        let max_dist_sq = st.config.preset.item_frame_cull_dist_sq(under_pressure);
        if block_dist_sq(cam_pos, event.pos) > max_dist_sq {
            st.saved_actions = st.saved_actions.saturating_add(1);
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    /// Suppresses non-player armor-stand / crowd name-tag overdraw in high-FPS presets or under
    /// active frame-time pressure. Never reveals hidden name tags or hides player tags.
    fn render_name_tag(&self, _: &dyn ClientHost, event: &mut RenderNameTagEvent<'_>) {
        if !event.visible {
            return;
        }
        let mut st = self.lock();
        if !st.config.enabled {
            return;
        }
        let under_pressure = st.frames.reading().under_pressure();
        if should_suppress_crowd_nametag(event.kind, event.visible, st.config.preset, under_pressure)
        {
            event.visible = false;
            st.saved_actions = st.saved_actions.saturating_add(1);
        }
    }

    /// Suppresses redundant ambient beacon swirls when the particle governor is active.
    /// Strictly never makes a hidden effect visible.
    fn effect_particle_modification(
        &self,
        _: &dyn Host,
        event: &mut EffectParticleModificationEvent<'_>,
    ) {
        if !event.visible {
            return;
        }
        let mut st = self.lock();
        if !st.config.enabled || !st.config.particle_governor {
            return;
        }
        let hide_ambient = match st.config.preset {
            Preset::Balanced => st.frames.reading().under_pressure() && event.effect.ambient,
            Preset::Competitive | Preset::MaxFps => event.effect.ambient,
        };
        if hide_ambient {
            event.visible = false;
            st.saved_actions = st.saved_actions.saturating_add(1);
        }
    }

    /// Samples authoritative server MSPT once per tick and resets per-tick explosion counters.
    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickStart {
            world_running: true,
        } = event.phase
        {
            let (_rate, avg_mspt) = host.tick_stats();
            let has_time = host.server_has_time();
            let (mut cfg, client_seen) = {
                let st = self.lock();
                (st.config, st.client_seen)
            };
            cfg.refresh_from_host(host, client_seen);

            let mut st = self.lock();
            st.config = cfg;
            st.server_tick_index = st.server_tick_index.wrapping_add(1);
            st.server_mspt = Some(avg_mspt);
            st.server_has_time = has_time;
            st.explosions_this_tick = 0;
        }
    }

    /// Deterministically phase-staggers ambient wildlife ticks only when server MSPT exceeds the
    /// preset's lag threshold.
    fn entity_tick_pre(&self, _: &mut dyn ServerHost, event: &mut EntityTickPreEvent<'_>) -> Flow {
        let mut st = self.lock();
        if !st.config.enabled || !st.config.tps_guard {
            return Flow::Continue;
        }
        let avg_mspt = st.server_mspt.unwrap_or(0.0);
        if should_pace_entity_tick(
            event.kind,
            event.entity,
            st.server_tick_index,
            avg_mspt,
            st.server_has_time,
            st.config.preset,
        ) {
            st.saved_actions = st.saved_actions.saturating_add(1);
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    /// Caps redundant ordinary-storage entity blast walks during multi-TNT chain reactions in the
    /// same server tick.
    fn explosion_detonate(&self, _: &mut dyn ServerHost, event: &mut ExplosionDetonateEvent<'_>) {
        let mut st = self.lock();
        if !st.config.enabled || !st.config.tps_guard {
            return;
        }
        st.explosions_this_tick = st.explosions_this_tick.saturating_add(1);
        let avg_mspt = st.server_mspt.unwrap_or(0.0);
        if let Some(cap) =
            explosion_entity_cap(st.explosions_this_tick, avg_mspt, st.config.preset)
            && event.entities.len() > cap
        {
            let trimmed = (event.entities.len() - cap) as u64;
            event.entities.truncate(cap);
            st.saved_actions = st.saved_actions.saturating_add(trimmed);
        }
    }

    /// Allows nearby experience orbs of differing values to merge when XP clumping is enabled.
    fn experience_orb_can_merge(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ExperienceOrbCanMergeEvent<'_>,
    ) {
        let st = self.lock();
        if st.config.enabled && st.config.xp_clumping {
            event.can_merge = true;
        }
    }

    /// Merges experience orbs using exact `u128` saved-NBT factorizations so zero XP is ever lost.
    fn experience_orb_merge(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ExperienceOrbMergeEvent<'_>,
    ) -> Flow {
        let active = {
            let st = self.lock();
            st.config.enabled && st.config.xp_clumping
        };
        if !active {
            return Flow::Continue;
        }
        match plan_orb_merge(
            event.into_value,
            event.into_count,
            event.from_value,
            event.from_count,
        ) {
            OrbMergePlan::LetEngineAddCounts => {
                let mut st = self.lock();
                st.saved_actions = st.saved_actions.saturating_add(1);
                Flow::Continue
            }
            OrbMergePlan::RewriteInto { value, count } => {
                if host.set_experience_orb(
                    event.dimension,
                    event.into,
                    Some(value),
                    Some(count),
                    None,
                ) {
                    let mut st = self.lock();
                    st.saved_actions = st.saved_actions.saturating_add(1);
                    Flow::Continue
                } else {
                    Flow::Cancel
                }
            }
            OrbMergePlan::Cancel => Flow::Cancel,
        }
    }

    /// Collapses multi-count clumped orbs on pickup and clears player XP pickup delay.
    fn pickup_xp(&self, host: &mut dyn ServerHost, event: &mut PickupXpEvent<'_>) -> Flow {
        let active = {
            let st = self.lock();
            st.config.enabled && st.config.xp_clumping
        };
        if !active {
            return Flow::Continue;
        }
        let (value, count) = resolve_orb_pickup(event.value, event.count);
        event.value = value;
        event.count = count;
        if let Some(player) = host.player(event.player) {
            let _ = player.set_exp_cooldown(host, 0);
        }
        Flow::Continue
    }

    /// Draws the optional corner mini-HUD, yielding automatically to F3's debug overlay.
    fn render_gui_post(&self, host: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
        let (cfg, reading, server_mspt, saved_actions) = {
            let st = self.lock();
            (
                st.config,
                st.frames.reading(),
                st.server_mspt,
                st.saved_actions,
            )
        };
        if !cfg.enabled || cfg.hud_style == HudStyle::Off {
            return;
        }
        if !host.native_corner_shown(cfg.hud_corner) {
            return;
        }
        let Some(text) = format_hud_text(cfg.hud_style, reading, server_mspt, saved_actions)
        else {
            return;
        };
        let ink = hud_ink_color(reading, server_mspt);
        let draw = &mut *event.draw;
        let [w, h] = box_size(draw.text_width(&text));
        let (x, y) = event.corners.place(cfg.hud_corner, w, h);
        draw.fill([x, y, w, h], HUD_BACKGROUND);
        draw.text(&text, [x + BOX_PADDING, y + BOX_PADDING], ink, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::{INK_BAD, INK_GOOD, INK_WARN};
    use crate::xp::orb_total_xp;

    #[test]
    fn all_registered_settings_pass_horizon_validation() {
        let specs = settings_specs();
        let mut keys = std::collections::HashSet::new();
        for spec in &specs {
            assert_eq!(spec.check(), Ok(()), "invalid setting spec: {}", spec.key);
            assert!(keys.insert(spec.key.clone()), "duplicate key: {}", spec.key);
            if matches!(spec.key.as_str(), "xp_clumping" | "tps_guard") {
                assert_eq!(
                    spec.kind,
                    mod_api::SettingKind::Toggle { default: false },
                    "{} must default to off",
                    spec.key
                );
            }
        }
        let defaults = ActiveConfig::default();
        assert!(!defaults.xp_clumping, "xp_clumping must default to false");
        assert!(!defaults.tps_guard, "tps_guard must default to false");
    }

    #[test]
    fn dynamic_fps_governor_respects_focus_minimize_and_menu_caps() {
        let mut cfg = ActiveConfig::default();
        cfg.unfocused_fps = 25;
        cfg.preset = Preset::Competitive;

        // Focused in world: keeps vanilla limit.
        assert_eq!(compute_fps_cap(240, true, false, true, &cfg), 240);
        // Focused on main menu in Competitive preset: caps idle menu to 120 FPS.
        assert_eq!(compute_fps_cap(260, true, false, false, &cfg), 120);
        // Unfocused window: caps to configured background limit (25 FPS), never raising lower vanilla limits.
        assert_eq!(compute_fps_cap(240, false, false, true, &cfg), 25);
        assert_eq!(compute_fps_cap(15, false, false, true, &cfg), 15);
        // Minimized window: drops to 1 FPS.
        assert_eq!(compute_fps_cap(240, false, true, true, &cfg), 1);
    }

    #[test]
    fn rolling_frame_tracker_computes_fps_and_1pct_low() {
        let mut tracker = FrameTracker::default();
        for i in 0..120 {
            // Inject two slow 50ms spikes (20 FPS equivalent) inside 120 frames at ~100 FPS (10ms).
            let dt = if i == 40 || i == 90 { 0.050 } else { 0.010 };
            tracker.record_frame(dt);
        }
        let r = tracker.reading();
        assert!((92..=98).contains(&r.fps), "unexpected fps: {r:?}");
        assert_eq!(r.min_fps, 20);
        assert_eq!(r.low_1pct, 20);
    }

    #[test]
    fn lossless_xp_orb_merging_preserves_exact_points_and_nbt_ranges() {
        let samples = [
            (3, 2, 5, 1),
            (17, 1, 37, 1),
            (32_767, 1, 32_767, 1),
            (32_767, 1, 5, 1),
            (10, 1, -4, 1),
            (-3, 2, 0, 1),
        ];
        for (iv, ic, fv, fc) in samples {
            match plan_orb_merge(iv, ic, fv, fc) {
                OrbMergePlan::RewriteInto { value, count } => {
                    assert!(i16::try_from(value).is_ok(), "value overflows i16: {value}");
                    assert!((1..=i32::MAX as u32).contains(&count));
                    assert_ne!((value, count), (iv, ic));
                    assert_eq!(
                        orb_total_xp(value, count),
                        orb_total_xp(iv, ic) + orb_total_xp(fv, fc)
                    );
                }
                OrbMergePlan::LetEngineAddCounts => {
                    assert_eq!(iv, 0);
                    assert_eq!(orb_total_xp(iv, ic) + orb_total_xp(fv, fc), 0);
                }
                OrbMergePlan::Cancel => panic!("expected representable merge for {iv},{ic}+{fv},{fc}"),
            }
        }
        // Unrepresentable overflow safely cancels instead of losing XP.
        assert_eq!(
            plan_orb_merge(32_767, i32::MAX as u32, 1, 1),
            OrbMergePlan::Cancel
        );
        assert_eq!(resolve_orb_pickup(5, 40), (200, 1));
    }

    #[test]
    fn adaptive_tps_guard_leaves_healthy_ticks_and_gameplay_entities_untouched() {
        // Healthy 12ms tick: nothing skipped.
        assert!(!should_pace_entity_tick(
            "minecraft:cod",
            7,
            100,
            12.0,
            true,
            Preset::Balanced
        ));
        // Overloaded 46ms tick: villagers, zombies, items, and players are never skipped.
        for critical in [
            "minecraft:player",
            "minecraft:villager",
            "minecraft:zombie",
            "minecraft:item",
            "minecraft:hopper_minecart",
        ] {
            assert!(!should_pace_entity_tick(
                critical,
                7,
                100,
                46.0,
                false,
                Preset::MaxFps
            ));
        }
        // Ambient fish alternate ticks deterministically under overload (tick N vs N+1).
        let t0 = should_pace_entity_tick("minecraft:cod", 7, 100, 44.0, false, Preset::Balanced);
        let t1 = should_pace_entity_tick("minecraft:cod", 7, 101, 44.0, false, Preset::Balanced);
        assert_ne!(t0, t1, "ambient entity must still tick on alternating phases");

        // Explosion cap only triggers on 4th+ explosion in a single tick or >=45mspt.
        assert_eq!(explosion_entity_cap(1, 15.0, Preset::Balanced), None);
        assert_eq!(explosion_entity_cap(4, 15.0, Preset::Competitive), Some(64));
        assert_eq!(explosion_entity_cap(14, 52.0, Preset::MaxFps), Some(24));
    }

    #[test]
    fn hud_formatting_and_severity_colors_behave_cleanly() {
        let smooth = crate::frame::FrameReading {
            fps: 144,
            min_fps: 110,
            low_1pct: 120,
            avg_ms: 6.9,
        };
        assert_eq!(format_hud_text(HudStyle::Off, smooth, Some(4.2), 10), None);
        assert_eq!(
            format_hud_text(HudStyle::Compact, smooth, Some(4.24), 10).as_deref(),
            Some("144 fps  4.2 mspt")
        );
        assert_eq!(
            format_hud_text(HudStyle::Detailed, smooth, Some(4.24), 1_500).as_deref(),
            Some("144 fps  1% 120  4.2 mspt  opt 1.5k")
        );
        assert_eq!(hud_ink_color(smooth, Some(5.0)), INK_GOOD);
        assert_eq!(hud_ink_color(smooth, Some(41.0)), INK_WARN);
        assert_eq!(hud_ink_color(smooth, Some(55.0)), INK_BAD);
    }
}
