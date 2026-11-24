//! Server TPS / MSPT adaptive entity pacing and chain-explosion blast-storm protection.

use crate::settings::Preset;

/// Ambient, non-farm wildlife whose AI/swim ticks can be safely phase-staggered during server
/// MSPT overload without affecting redstone, villagers, hostile mob farms, bosses, or players.
#[inline]
pub fn is_ambient_wildlife(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:bat"
            | "minecraft:cod"
            | "minecraft:salmon"
            | "minecraft:tropical_fish"
            | "minecraft:tadpole"
            | "minecraft:squid"
            | "minecraft:glow_squid"
    )
}

/// Decides whether `entity` of type `kind` should skip this single tick (`Flow::Cancel`) under
/// active server MSPT pressure.
///
/// When the server tick budget is healthy, this returns `false` immediately so every entity ticks
/// identically to vanilla. Under overload, ambient wildlife (and under severe >48ms stalls,
/// experience orbs) are deterministically phase-staggered across ticks using `(entity + tick)` so
/// no entity ever freezes permanently.
#[inline]
pub fn should_pace_entity_tick(
    kind: &str,
    entity_id: u32,
    tick_index: u64,
    avg_mspt: f64,
    server_has_time: bool,
    preset: Preset,
) -> bool {
    let threshold = preset.mspt_guard_threshold();
    let overloaded = avg_mspt >= threshold || (!server_has_time && avg_mspt >= threshold * 0.85);
    if !overloaded {
        return false;
    }

    let phase = (entity_id as u64).wrapping_add(tick_index);
    if is_ambient_wildlife(kind) {
        // Under severe lag (>48 mspt), tick ambient fish/bats once every 4 ticks; otherwise every 2.
        let mask = if avg_mspt >= 48.0 { 0b11 } else { 0b1 };
        return (phase & mask) != 0;
    }

    // Under severe server overload (>48 mspt), tick experience orbs every 2nd tick (merging still runs).
    if avg_mspt >= 48.0 && kind == "minecraft:experience_orb" {
        return (phase & 0b1) != 0;
    }

    false
}

/// Computes an optional cap on `ExplosionDetonateEvent::entities` during same-tick TNT/creeper
/// chain reactions or high MSPT spikes.
///
/// Note: Horizon runs players, end crystals, and the ender dragon in separate earlier passes, and
/// trailing decorations/items last in storage order, so capping ordinary-storage entity walks on
/// the 4th+ explosion in a single tick prevents O(explosions * entities) stalls while keeping
/// primary explosions untouched.
#[inline]
pub fn explosion_entity_cap(
    explosions_this_tick: u32,
    avg_mspt: f64,
    preset: Preset,
) -> Option<usize> {
    if explosions_this_tick >= 4 || avg_mspt >= 45.0 {
        let base = preset.explosion_storm_entity_cap();
        // Tighten further if >12 explosions detonate within the exact same 50ms tick.
        if explosions_this_tick >= 12 {
            Some((base / 2).max(16))
        } else {
            Some(base)
        }
    } else {
        None
    }
}
