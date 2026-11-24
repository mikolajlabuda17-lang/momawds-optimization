//! Corner mini-HUD layout, text formatting, and severity color coding.

use crate::frame::FrameReading;
use crate::settings::HudStyle;

/// Translucent dark HUD pill matching Horizon's native overlay style (`0x7800_0000`).
pub const HUD_BACKGROUND: u32 = 0x7800_0000;
/// Crisp white text (`#FFFFFF`) when frametimes and MSPT are healthy.
pub const INK_GOOD: u32 = 0xFFFF_FFFF;
/// Warm amber text (`#FFD166`) when FPS < 45 or server MSPT > 38 ms.
pub const INK_WARN: u32 = 0xFFFF_D166;
/// Soft coral text (`#FF6B6B`) when FPS < 25 or server MSPT >= 50 ms (< 20 TPS).
pub const INK_BAD: u32 = 0xFFFF_6B6B;

/// Box height and text inset in GUI pixels.
pub const BOX_HEIGHT: f32 = 12.0;
pub const BOX_PADDING: f32 = 2.0;

/// Computes `[width, height]` of the HUD background rectangle for text of pixel width `text_w`.
#[inline]
pub fn box_size(text_w: f32) -> [f32; 2] {
    [text_w + 2.0 * BOX_PADDING, BOX_HEIGHT]
}

/// Selects HUD text ink color from current FPS and integrated-server MSPT.
#[inline]
pub fn hud_ink_color(reading: FrameReading, server_mspt: Option<f64>) -> u32 {
    let mspt = server_mspt.unwrap_or(0.0);
    if (reading.fps > 0 && reading.fps < 25) || mspt >= 50.0 {
        INK_BAD
    } else if reading.under_pressure() || mspt >= 38.0 {
        INK_WARN
    } else {
        INK_GOOD
    }
}

/// Formats the HUD string for `Compact` or `Detailed` mode (`None` when `HudStyle::Off`).
pub fn format_hud_text(
    style: HudStyle,
    reading: FrameReading,
    server_mspt: Option<f64>,
    saved_actions: u64,
) -> Option<String> {
    match style {
        HudStyle::Off => None,
        HudStyle::Compact => match server_mspt {
            Some(mspt) if mspt > 0.05 => Some(format!("{} fps  {mspt:.1} mspt", reading.fps)),
            _ => Some(format!("{} fps", reading.fps)),
        },
        HudStyle::Detailed => {
            let mut line = format!("{} fps  1% {}", reading.fps, reading.low_1pct);
            if let Some(mspt) = server_mspt
                && mspt > 0.05
            {
                line.push_str(&format!("  {mspt:.1} mspt"));
            }
            if saved_actions > 0 {
                line.push_str(&format!("  opt {}", format_compact_count(saved_actions)));
            }
            Some(line)
        }
    }
}

fn format_compact_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
