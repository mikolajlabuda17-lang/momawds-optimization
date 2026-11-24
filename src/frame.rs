//! Rolling client frame-time counter, dynamic framerate limiter, and distance culling math.

use std::collections::VecDeque;

use crate::settings::{ActiveConfig, Preset};

/// Lookback window in seconds for slowest-frame and 1% low calculations.
const LOW_WINDOW_SECS: f64 = 5.0;
/// Minimum interval between percentile recomputations so frame hooks stay allocation-light.
const REFRESH_INTERVAL_SECS: f64 = 0.5;

/// Snapshot of recent client framerate performance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameReading {
    /// Frames completed in the last 1.0 second.
    pub fps: u32,
    /// Slowest single frame in the last 5.0 seconds, expressed as equivalent FPS.
    pub min_fps: u32,
    /// 99th-percentile frame time in the last 5.0 seconds, expressed as equivalent 1% low FPS.
    pub low_1pct: u32,
    /// Mean frame duration in milliseconds over the last 1.0 second.
    pub avg_ms: f32,
}

impl FrameReading {
    /// True when the client is actively experiencing low-FPS pressure (< 45 FPS or 1% low < 30).
    pub fn under_pressure(self) -> bool {
        self.fps > 0 && (self.fps < 45 || self.low_1pct < 30)
    }
}

/// Rolling 5-second frame counter.
#[derive(Debug)]
pub struct FrameTracker {
    clock_secs: f64,
    frames: VecDeque<(f64, f32)>,
    scratch: Vec<f32>,
    reading: FrameReading,
    computed_at: f64,
}

impl Default for FrameTracker {
    fn default() -> Self {
        Self {
            clock_secs: 0.0,
            frames: VecDeque::with_capacity(512),
            scratch: Vec::with_capacity(512),
            reading: FrameReading::default(),
            computed_at: f64::NEG_INFINITY,
        }
    }
}

impl FrameTracker {
    /// Records one frame of duration `elapsed_secs`.
    pub fn record_frame(&mut self, elapsed_secs: f64) {
        if !elapsed_secs.is_finite() || elapsed_secs <= 0.0 {
            return;
        }
        let clamped = elapsed_secs.min(2.0);
        self.clock_secs += clamped;
        let now = self.clock_secs;
        self.frames.push_back((now, (clamped * 1000.0) as f32));
        while self
            .frames
            .front()
            .is_some_and(|&(t, _)| now - t > LOW_WINDOW_SECS)
        {
            self.frames.pop_front();
        }
        if now - self.computed_at >= REFRESH_INTERVAL_SECS {
            self.computed_at = now;
            self.recompute();
        }
    }

    fn recompute(&mut self) {
        self.scratch.clear();
        let mut recent_count = 0u32;
        let mut recent_ms_sum = 0.0f32;
        for &(t, ms) in &self.frames {
            self.scratch.push(ms);
            if self.clock_secs - t <= 1.0 {
                recent_count += 1;
                recent_ms_sum += ms;
            }
        }
        if self.scratch.is_empty() {
            self.reading = FrameReading::default();
            return;
        }
        self.scratch.sort_unstable_by(|a, b| b.total_cmp(a));
        let to_fps = |ms: f32| (1000.0 / ms.max(0.05)).round() as u32;
        let idx_1pct = (self.scratch.len() - 1) / 100;
        let avg_ms = if recent_count > 0 {
            recent_ms_sum / recent_count as f32
        } else {
            self.scratch[self.scratch.len() / 2]
        };
        self.reading = FrameReading {
            fps: recent_count,
            min_fps: to_fps(self.scratch[0]),
            low_1pct: to_fps(self.scratch[idx_1pct]),
            avg_ms,
        };
    }

    pub fn reading(&self) -> FrameReading {
        self.reading
    }
}

/// Computes the target framerate limit given window focus/minimization and current config.
/// Horizon keeps the lower of its own limit and the value written back, ignoring `< 1`.
pub fn compute_fps_cap(
    current_limit: i32,
    focused: bool,
    minimized: bool,
    in_world: bool,
    cfg: &ActiveConfig,
) -> i32 {
    if !cfg.enabled {
        return current_limit;
    }
    if minimized {
        return current_limit.min(1).max(1);
    }
    if !focused {
        return current_limit.min(cfg.unfocused_fps.max(1));
    }
    if !in_world
        && let Some(menu_cap) = cfg.preset.menu_idle_fps_cap()
    {
        return current_limit.min(menu_cap);
    }
    current_limit
}

/// Squared Euclidean distance from the camera to the center of block `pos`.
#[inline]
pub fn block_dist_sq(camera_pos: [f64; 3], pos: [i32; 3]) -> f64 {
    let dx = (pos[0] as f64 + 0.5) - camera_pos[0];
    let dy = (pos[1] as f64 + 0.5) - camera_pos[1];
    let dz = (pos[2] as f64 + 0.5) - camera_pos[2];
    dx * dx + dy * dy + dz * dz
}

/// Decides whether a non-player crowd name tag should be suppressed during heavy clutter.
/// Never reveals a hidden name tag (`vanilla_visible == false`), and never hides player tags.
#[inline]
pub fn should_suppress_crowd_nametag(
    kind: &str,
    currently_visible: bool,
    preset: Preset,
    under_pressure: bool,
) -> bool {
    if !currently_visible || kind == "minecraft:player" {
        return false;
    }
    match preset {
        Preset::Balanced => false,
        Preset::Competitive => under_pressure && kind == "minecraft:armor_stand",
        Preset::MaxFps => under_pressure || kind == "minecraft:armor_stand",
    }
}
