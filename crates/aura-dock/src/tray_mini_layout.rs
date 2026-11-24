//! Compact notification-area geometry, independent of Windows APIs.
pub const MAX_ICONS: usize = 7;
pub const SLOT: f32 = 26.0;
pub const CONTROL: f32 = 42.0;
pub const PAD: f32 = 5.0;
pub const HEIGHT: f32 = 36.0;

pub fn icon_capacity(monitor_width: i32, scale: f32) -> usize {
    let logical = (monitor_width.max(1) as f32 / scale.max(0.25)) - 16.0;
    // Reserve the control button, chevron, padding, and a margin for rounding.
    (((logical - CONTROL - SLOT - 2.0 * PAD - 2.0) / SLOT).floor().max(0.0) as usize).min(MAX_ICONS)
}
pub fn width(count: usize, overflow: bool, scale: f32) -> i32 {
    ((2.0 * PAD + CONTROL + (count + usize::from(overflow)) as f32 * SLOT) * scale).ceil() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserves_room_for_button_and_overflow_on_small_or_scaled_screens() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for w in [200, 320, 800, 1920] {
                let count = icon_capacity(w, scale);
                assert!(count <= MAX_ICONS);
                assert!(width(count, true, scale) <= w - (16.0 * scale) as i32);
            }
        }
    }
}
