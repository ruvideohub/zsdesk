//! Pure decoration layout; no changes to title-bar hit testing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Badge {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A count badge is optional when the header is too narrow. Values are pixels,
/// `scale` is the monitor DPI divided by 96.
pub fn count_badge(width: f32, height: f32, scale: f32) -> Option<Badge> {
    if !width.is_finite()
        || !height.is_finite()
        || !scale.is_finite()
        || scale <= 0.0
        || width < 140.0 * scale
        || height < 20.0 * scale
    {
        return None;
    }
    Some(Badge {
        x: width - 42.0 * scale,
        y: (height - 20.0 * scale) / 2.0,
        width: 34.0 * scale,
        height: 20.0 * scale,
    })
}
/// Symmetric title inset reserves room for the lock and the optional badge.
pub fn title_side(scale: f32, hovered: bool, has_badge: bool, locked: bool) -> f32 {
    if hovered || (has_badge && locked) {
        66.0 * scale
    } else if has_badge {
        46.0 * scale
    } else {
        36.0 * scale
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn count_badge_stays_inside_header_and_away_from_title() {
        for k in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            let w = 200.0 * k;
            let h = 36.0 * k;
            let b = count_badge(w, h, k).unwrap();
            assert!(
                b.x >= w - title_side(k, false, true, false)
                    && b.x + b.width <= w
                    && b.y >= 0.0
                    && b.y + b.height <= h
            );
            // A maximally wide locked title has a 16-DIP lock reservation,
            // followed by a 4-DIP gap and a 12-DIP glyph.
            let rightmost_lock_edge = w - title_side(k, false, true, true) + (8.0 + 4.0 + 12.0) * k;
            assert!(rightmost_lock_edge <= b.x + 0.001);
            assert!(count_badge(139.0 * k, h, k).is_none());
        }
        assert!(count_badge(f32::NAN, 36.0, 1.0).is_none());
        assert!(count_badge(200.0, 10.0, 1.0).is_none());
        assert!(count_badge(200.0, 36.0, 0.0).is_none());
    }
}
