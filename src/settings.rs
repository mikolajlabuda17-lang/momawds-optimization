//! Settings definitions, performance presets, and fast in-memory resolved configuration.

use mod_api::{ClientHost, Corner, Host, SettingSpec};

/// Mod identifier matching `horizon-mod.toml`.
pub const MOD_ID: &str = "momawds_optimization";

/// Screen corners in choice-index order.
pub const CORNERS: [(Corner, &str); 4] = [
    (Corner::TopLeft, "top_left"),
    (Corner::TopRight, "top_right"),
    (Corner::BottomLeft, "bottom_left"),
    (Corner::BottomRight, "bottom_right"),
];

/// Baseline performance profile selected in `Options > Mods > Momawd's Optimization Mod`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    #[default]
    Balanced = 0,
    Competitive = 1,
    MaxFps = 2,
}

impl Preset {
    pub fn from_index(idx: u32) -> Self {
        match idx {
            1 => Self::Competitive,
            2 => Self::MaxFps,
            _ => Self::Balanced,
        }
    }

    /// Squared distance (blocks²) beyond which 3D items inside item frames are culled.
    pub fn item_frame_cull_dist_sq(self, low_fps_pressure: bool) -> f64 {
        let base_blocks = match self {
            Self::Balanced => 48.0,
            Self::Competitive => 32.0,
            Self::MaxFps => 18.0,
        };
        let scaled = if low_fps_pressure {
            base_blocks * 0.65
        } else {
            base_blocks
        };
        scaled * scaled
    }

    /// Idle main-menu FPS ceiling while focused outside a world (`None` leaves vanilla limit).
    pub fn menu_idle_fps_cap(self) -> Option<i32> {
        match self {
            Self::Balanced => None,
            Self::Competitive => Some(120),
            Self::MaxFps => Some(60),
        }
    }

    /// Server MSPT threshold above which adaptive ambient entity pacing activates.
    pub fn mspt_guard_threshold(self) -> f64 {
        match self {
            Self::Balanced => 42.0,
            Self::Competitive => 34.0,
            Self::MaxFps => 26.0,
        }
    }

    /// Maximum ordinary-storage entities walked per explosion during multi-explosion ticks.
    pub fn explosion_storm_entity_cap(self) -> usize {
        match self {
            Self::Balanced => 96,
            Self::Competitive => 64,
            Self::MaxFps => 48,
        }
    }
}

/// HUD display verbosity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HudStyle {
    Off = 0,
    #[default]
    Compact = 1,
    Detailed = 2,
}

impl HudStyle {
    pub fn from_index(idx: u32) -> Self {
        match idx {
            0 => Self::Off,
            2 => Self::Detailed,
            _ => Self::Compact,
        }
    }
}

/// Fast `Copy` snapshot of user settings cached once per frame/tick so hot-path hooks
/// never make repeated WASM host calls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveConfig {
    pub enabled: bool,
    pub preset: Preset,
    pub unfocused_fps: i32,
    pub item_frame_cull: bool,
    pub particle_governor: bool,
    pub xp_clumping: bool,
    pub tps_guard: bool,
    pub hud_style: HudStyle,
    pub hud_corner: Corner,
}

impl Default for ActiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            preset: Preset::Balanced,
            unfocused_fps: 30,
            item_frame_cull: true,
            particle_governor: true,
            xp_clumping: false,
            tps_guard: false,
            hud_style: HudStyle::Compact,
            hud_corner: Corner::TopLeft,
        }
    }
}

impl ActiveConfig {
    /// Reads current settings from a `ClientHost` (once per frame).
    pub fn from_client(host: &dyn ClientHost) -> Self {
        let enabled = host.on(MOD_ID);
        let preset = Preset::from_index(host.setting_choice("preset").unwrap_or(0));
        let unfocused_fps = host
            .setting_f64("unfocused_fps")
            .unwrap_or(30.0)
            .round()
            .clamp(5.0, 60.0) as i32;
        let item_frame_cull = host.setting_bool("item_frame_cull").unwrap_or(true);
        let particle_governor = host.setting_bool("particle_governor").unwrap_or(true);
        let xp_clumping = host.setting_bool("xp_clumping").unwrap_or(false);
        let tps_guard = host.setting_bool("tps_guard").unwrap_or(false);
        let hud_style = HudStyle::from_index(host.setting_choice("hud_mode").unwrap_or(1));
        let corner_idx = host
            .setting_choice("hud_corner")
            .unwrap_or(0)
            .min((CORNERS.len() - 1) as u32) as usize;
        Self {
            enabled,
            preset,
            unfocused_fps,
            item_frame_cull,
            particle_governor,
            xp_clumping,
            tps_guard,
            hud_style,
            hud_corner: CORNERS[corner_idx].0,
        }
    }

    /// Updates server-relevant toggles when running on a dedicated server where `ClientHost`
    /// never runs; preserves client-resolved values in singleplayer integrated worlds.
    pub fn refresh_from_host(&mut self, host: &dyn Host, client_active: bool) {
        self.enabled = host.on(MOD_ID);
        if client_active {
            return;
        }
        self.preset = host
            .setting(MOD_ID, "preset")
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .map(Preset::from_index)
            .unwrap_or(Preset::Balanced);
        self.particle_governor = host
            .setting(MOD_ID, "particle_governor")
            .and_then(|raw| raw.trim().parse::<bool>().ok())
            .unwrap_or(true);
        self.xp_clumping = host
            .setting(MOD_ID, "xp_clumping")
            .and_then(|raw| raw.trim().parse::<bool>().ok())
            .unwrap_or(false);
        self.tps_guard = host
            .setting(MOD_ID, "tps_guard")
            .and_then(|raw| raw.trim().parse::<bool>().ok())
            .unwrap_or(false);
    }
}

/// All settings registered on `Options > Mods > Momawd's Optimization Mod`.
pub fn settings_specs() -> [SettingSpec; 8] {
    [
        SettingSpec::choice(
            "preset",
            "momawds_optimization.setting.preset",
            [
                "momawds_optimization.preset.balanced",
                "momawds_optimization.preset.competitive",
                "momawds_optimization.preset.max_fps",
            ],
            0,
        )
        .tooltip("momawds_optimization.setting.preset.tooltip"),
        SettingSpec::range(
            "unfocused_fps",
            "momawds_optimization.setting.unfocused_fps",
            5.0,
            60.0,
            5.0,
            30.0,
        )
        .tooltip("momawds_optimization.setting.unfocused_fps.tooltip"),
        SettingSpec::toggle(
            "item_frame_cull",
            "momawds_optimization.setting.item_frame_cull",
            true,
        )
        .tooltip("momawds_optimization.setting.item_frame_cull.tooltip"),
        SettingSpec::toggle(
            "particle_governor",
            "momawds_optimization.setting.particle_governor",
            true,
        )
        .tooltip("momawds_optimization.setting.particle_governor.tooltip"),
        SettingSpec::toggle(
            "xp_clumping",
            "momawds_optimization.setting.xp_clumping",
            false,
        )
        .tooltip("momawds_optimization.setting.xp_clumping.tooltip"),
        SettingSpec::toggle(
            "tps_guard",
            "momawds_optimization.setting.tps_guard",
            false,
        )
        .tooltip("momawds_optimization.setting.tps_guard.tooltip"),
        SettingSpec::choice(
            "hud_mode",
            "momawds_optimization.setting.hud_mode",
            [
                "momawds_optimization.hud.off",
                "momawds_optimization.hud.compact",
                "momawds_optimization.hud.detailed",
            ],
            1,
        )
        .tooltip("momawds_optimization.setting.hud_mode.tooltip"),
        SettingSpec::choice(
            "hud_corner",
            "momawds_optimization.setting.hud_corner",
            CORNERS.map(|(_, name)| format!("momawds_optimization.corner.{name}")),
            0,
        )
        .tooltip("momawds_optimization.setting.hud_corner.tooltip"),
    ]
}
