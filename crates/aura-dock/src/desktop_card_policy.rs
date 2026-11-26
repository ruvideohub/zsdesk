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
