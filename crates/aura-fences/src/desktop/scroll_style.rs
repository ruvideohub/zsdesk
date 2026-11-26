//! Presentation-only scrollbar policy. Hit testing remains in `Geometry`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    pub thumb_opacity: f32,
    pub show_track: bool,
}

/// A faint idle thumb makes overflowing contents discoverable. High contrast
/// always uses the full system text color rather than translucent blends.
pub fn appearance(hovered: bool, dragging: bool, high_contrast: bool) -> Appearance {
    Appearance {
        thumb_opacity: if high_contrast {
            1.0
        } else if dragging {
            0.95
        } else if hovered {
            0.75
        } else {
            0.42
        },
        show_track: hovered || dragging,
    }
}

/// Height of the existing track inside its content viewport, in pixels.
pub fn track_height(content_height: f32, scale: f32) -> Option<f32> {
    if !content_height.is_finite() || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let height = content_height - 8.0 * scale;
    (height > 0.0).then_some(height)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_and_active_thumb_respect_high_contrast() {
        let idle = appearance(false, false, false);
        let hover = appearance(true, false, false);
        let drag = appearance(false, true, false);
        assert!(
            idle.thumb_opacity > 0.0
                && idle.thumb_opacity < hover.thumb_opacity
                && hover.thumb_opacity < drag.thumb_opacity
        );
        assert!(!idle.show_track && hover.show_track && drag.show_track);
        for hover in [false, true] {
            for drag in [false, true] {
                assert_eq!(appearance(hover, drag, true).thumb_opacity, 1.0);
            }
        }
    }
    #[test]
    fn track_never_extends_beyond_its_content() {
        for k in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            assert_eq!(track_height(8.0 * k, k), None);
            let h = 100.0 * k;
            let track = track_height(h, k).unwrap();
            assert!(4.0 * k + track <= h - 4.0 * k + 0.001);
        }
        assert_eq!(track_height(f32::NAN, 1.0), None);
        assert_eq!(track_height(100.0, 0.0), None);
    }
}
