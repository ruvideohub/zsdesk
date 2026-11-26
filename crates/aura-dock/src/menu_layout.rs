//! Pure viewport and scrolling rules for the Dock's Composition menus.

/// Reserve space for the panel shadow; a normal-sized monitor keeps the entire popup inside.
pub fn viewport(content: f32, screen: f32, shadow_margin: f32) -> f32 {
    content.max(1.0).min((screen - 2.0 * shadow_margin).max(1.0))
}

pub fn max_scroll(content: f32, viewport: f32) -> f32 {
    (content - viewport).max(0.0)
}

/// Scroll the minimum needed to show a keyboard-selected row, including the padding.
pub fn reveal(scroll: f32, row_y: f32, row_h: f32, viewport: f32, content: f32, pad: f32) -> f32 {
    let max = max_scroll(content, viewport);
    let pad = pad.max(0.0).min((viewport - row_h).max(0.0) / 2.0);
    let top = row_y - pad;
    let bottom = row_y + row_h + pad - viewport;
    let scroll = if scroll > top { top } else if scroll < bottom { bottom } else { scroll };
    scroll.clamp(0.0, max)
}

/// Reject shadow and clipped portions before translating into content coordinates.
pub fn hit_y(y: f32, origin: f32, viewport: f32, scroll: f32) -> Option<f32> {
    let local = y - origin;
    if local >= 0.0 && local < viewport { Some(local + scroll) } else { None }
}

/// A proportional thumb that never leaves its track even for very long menus.
pub fn thumb(viewport: f32, content: f32, scroll: f32, pad: f32, minimum: f32) -> (f32, f32) {
    let track = (viewport - 2.0 * pad).max(0.0);
    let height = if content > 0.0 { (track * viewport / content).clamp(minimum.min(track), track) } else { track };
    let max = max_scroll(content, viewport);
    let ratio = if max > 0.0 { (scroll / max).clamp(0.0, 1.0) } else { 0.0 };
    (pad + (track - height) * ratio, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_and_keyboard_reveal_work_at_all_supported_dpis() {
        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            let row = 26.0 * scale;
            let content = 40.0 * row + 10.0 * scale;
            let screen = 600.0 * scale;
            let shadow = 28.0 * scale;
            let view = viewport(content, screen, shadow);
            assert!(view + 2.0 * shadow <= screen + 0.01);
            let selected = 37.0 * row + 5.0 * scale;
            let offset = reveal(0.0, selected, row, view, content, 5.0 * scale);
            assert!(selected >= offset && selected + row <= offset + view);
            assert_eq!(reveal(offset, selected, row, view, content, 5.0 * scale), offset);
            assert_eq!(reveal(offset, 5.0 * scale, row, view, content, 5.0 * scale), 0.0);
        }
    }
    #[test]
    fn pointer_hits_are_clipped_and_account_for_scroll() {
        assert_eq!(hit_y(9.0, 10.0, 100.0, 30.0), None);
        assert_eq!(hit_y(10.0, 10.0, 100.0, 30.0), Some(30.0));
        assert_eq!(hit_y(109.0, 10.0, 100.0, 30.0), Some(129.0));
        assert_eq!(hit_y(110.0, 10.0, 100.0, 30.0), None);
        assert_eq!(hit_y(f32::NAN, 10.0, 100.0, 30.0), None);
    }
    #[test]
    fn thumb_never_escapes_track() {
        for scroll in [0.0, 450.0, 900.0, 1000.0] {
            let (y, h) = thumb(100.0, 1000.0, scroll, 5.0, 16.0);
            assert!(y >= 5.0 && y + h <= 95.0);
            assert!(h >= 16.0);
        }
        let (y, h) = thumb(20.0, 500.0, 0.0, 5.0, 16.0);
        assert_eq!((y, h), (5.0, 10.0));
    }
}
