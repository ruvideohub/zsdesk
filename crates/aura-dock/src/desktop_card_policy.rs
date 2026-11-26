//! Pure desktop-card appearance rules, shared by every desktop widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Environment {
    pub light: bool,
    pub high_contrast: bool,
    pub transparency: bool,
    pub system_colors: [u32; 3],
}
impl Environment {
    /// Glass requires both a usable backdrop and the user's transparency setting.
    pub fn glass(self, backdrop_available: bool) -> bool {
        backdrop_available && self.transparency && !self.high_contrast
    }
}
/// The actual width of a meter fill. Unknown, invalid and zero readings do not
/// invent a coloured value; small positive values get a visible rounded cap.
pub fn meter_width(fill: f32, track: f32, minimum: f32) -> Option<f32> {
    if !fill.is_finite()
        || !track.is_finite()
        || !minimum.is_finite()
        || fill <= 0.0
        || track <= 0.0
    {
        return None;
    }
    Some(
        (track * fill.clamp(0.0, 1.0))
            .max(minimum.max(0.0).min(track))
            .min(track),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glass_never_overrides_accessibility_or_a_missing_backdrop() {
        let mut e = Environment {
            light: false,
            high_contrast: false,
            transparency: true,
            system_colors: [0; 3],
        };
        assert!(e.glass(true));
        assert!(!e.glass(false));
        e.transparency = false;
        assert!(!e.glass(true));
        e.transparency = true;
        e.high_contrast = true;
        assert!(!e.glass(true));
    }
    #[test]
    fn meter_stays_inside_its_track_and_does_not_fake_readings() {
        for f in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            assert_eq!(meter_width(f, 100.0, 5.0), None);
        }
        assert_eq!(meter_width(0.001, 100.0, 5.0), Some(5.0));
        assert_eq!(meter_width(0.5, 100.0, 5.0), Some(50.0));
        assert_eq!(meter_width(2.0, 100.0, 5.0), Some(100.0));
        assert_eq!(meter_width(1.0, 3.0, 5.0), Some(3.0));
        assert_eq!(meter_width(1.0, 0.0, 5.0), None);
    }
}

/// Shrink long values before using ellipsis; retain a readable minimum size.
/// Widths include the small rendering margin returned by the text surface.
pub fn fitted_font_size(font: f32, natural_width: f32, available: f32) -> f32 {
    if !font.is_finite() || font <= 0.0 {
        return 1.0;
    }
    if !natural_width.is_finite()
        || natural_width <= 0.0
        || !available.is_finite()
        || available <= 0.0
    {
        return font;
    }
    if natural_width <= available {
        return font;
    }
    (font * (available - 3.0).max(1.0) / natural_width).clamp(font * 0.70, font)
}

#[cfg(test)]
mod fit_tests {
    use super::fitted_font_size;
    #[test]
    fn time_and_date_shrink_only_as_needed_and_stay_readable() {
        assert_eq!(fitted_font_size(49.0, 150.0, 180.0), 49.0);
        assert!(fitted_font_size(49.0, 220.0, 180.0) < 49.0);
        assert_eq!(fitted_font_size(10.0, 1000.0, 50.0), 7.0);
        assert_eq!(fitted_font_size(10.0, f32::NAN, 50.0), 10.0);
        assert_eq!(fitted_font_size(f32::NAN, 100.0, 50.0), 1.0);
        for scale in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            let fit = fitted_font_size(36.0 * scale, 210.0 * scale, 182.0 * scale);
            assert!(fit >= 36.0 * scale * 0.70 && fit <= 36.0 * scale);
        }
    }
}
