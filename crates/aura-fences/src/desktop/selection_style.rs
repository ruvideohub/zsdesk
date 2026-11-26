//! Geometry for the focused item, separate from selection and hit testing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub inset: f32,
    pub radius: f32,
    pub width: f32,
}

/// An inner focus ring must remain within the selected cell even at high DPI.
/// Tiny/invalid cells keep their existing selection outline instead.
pub fn focus_ring(cell_width: f32, cell_height: f32, radius: f32, scale: f32) -> Option<Ring> {
    if !cell_width.is_finite()
        || !cell_height.is_finite()
        || !radius.is_finite()
        || !scale.is_finite()
        || scale <= 0.0
        || cell_width < 16.0 * scale
        || cell_height < 16.0 * scale
    {
        return None;
    }
    let inset = 3.0 * scale;
    let inner_w = cell_width - 2.0 * inset;
    let inner_h = cell_height - 2.0 * inset;
    Some(Ring {
        inset,
        radius: (radius - inset).max(0.0).min(inner_w.min(inner_h) / 2.0),
        width: scale.max(1.0).min(inner_w.min(inner_h)),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inner_ring_stays_inside_cells_at_supported_dpi() {
        for k in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            for (w, h) in [(16.0, 16.0), (90.0, 26.0), (110.0, 98.0)] {
                let (w, h) = (w * k, h * k);
                let r = focus_ring(w, h, 8.0 * k, k).unwrap();
                assert!(r.inset > 0.0 && r.inset * 2.0 < w.min(h));
                assert!(
                    r.radius >= 0.0 && r.radius <= (w - 2.0 * r.inset).min(h - 2.0 * r.inset) / 2.0
                );
                assert!(r.width > 0.0 && r.width <= (w - 2.0 * r.inset).min(h - 2.0 * r.inset));
            }
            assert!(focus_ring(15.0 * k, 30.0 * k, 8.0 * k, k).is_none());
        }
        assert!(focus_ring(f32::NAN, 20.0, 8.0, 1.0).is_none());
        assert!(focus_ring(20.0, 20.0, 8.0, 0.0).is_none());
    }
}
