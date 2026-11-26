//! Pure empty-state geometry, relative to the existing content viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmptyState {
    pub icon_tile: Rect,
    pub message: Rect,
}

/// Use an illustration only when it and the complete message row fit. The
/// original text-only layout remains the fallback for small panels and errors.
pub fn layout(width: f32, height: f32, scale: f32) -> Option<EmptyState> {
    if !width.is_finite()
        || !height.is_finite()
        || !scale.is_finite()
        || scale <= 0.0
        || width < 160.0 * scale
        || height < 80.0 * scale
    {
        return None;
    }
    let tile = 34.0 * scale;
    let gap = 8.0 * scale;
    let message_h = 26.0 * scale;
    let y = (height - tile - gap - message_h) / 2.0;
    Some(EmptyState {
        icon_tile: Rect {
            x: (width - tile) / 2.0,
            y,
            w: tile,
            h: tile,
        },
        message: Rect {
            x: 0.0,
            y: y + tile + gap,
            w: width,
            h: message_h,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_art_never_crops_or_obstructs_the_message() {
        for s in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            for (w, h) in [(160.0, 80.0), (200.0, 100.0), (640.0, 500.0)] {
                let (w, h) = (w * s, h * s);
                let x = layout(w, h, s).unwrap();
                assert!(x.icon_tile.x >= 0.0 && x.icon_tile.x + x.icon_tile.w <= w);
                assert!(x.icon_tile.y >= 0.0 && x.icon_tile.y + x.icon_tile.h <= x.message.y);
                assert!(x.message.x >= 0.0 && x.message.x + x.message.w <= w);
                assert!(x.message.y + x.message.h <= h + 0.001);
            }
            assert!(layout(159.0 * s, 80.0 * s, s).is_none());
            assert!(layout(160.0 * s, 79.0 * s, s).is_none());
        }
        assert!(layout(f32::NAN, 100.0, 1.0).is_none());
        assert!(layout(200.0, 100.0, 0.0).is_none());
    }
}
