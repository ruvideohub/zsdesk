//! macOS-style magnification curve.
//!
//! The dock scales an icon by its distance to the pointer: the hovered icon is
//! at `max`, the neighbours fall off along a cosine bell inside `influence`
//! icon pitches. The curve is pure math so it can be unit-tested without a
//! window, and it is evaluated on every pointer move instead of being
//! pre-baked into a lookup table.

/// Cosine bell on `t` in `0..=1`; `1` under the pointer, `0` at the edge of
/// the influence radius. Outside the radius the result is exactly `0`.
pub fn bell(t: f32) -> f32 {
    if !t.is_finite() || t >= 1.0 { return 0.0; }
    if t <= 0.0 { return 1.0; }
    0.5 * (1.0 + (std::f32::consts::PI * t).cos())
}

/// Scale factor for an icon whose center is `distance` pixels from the pointer.
///
/// `influence` is the radius in pixels; `max` is the scale right under the
/// pointer. Both are clamped so a corrupt configuration cannot draw a dock
/// that never fits on screen.
pub fn scale(distance: f32, influence: f32, max: f32) -> f32 {
    let max = max.clamp(1.0, 3.0);
    if (max - 1.0).abs() < f32::EPSILON { return 1.0; }
    let influence = influence.max(1.0);
    1.0 + (max - 1.0) * bell(distance.abs() / influence)
}

/// Eases `current` towards `target` by at most `step` (frame-rate independent
/// enough for a 60 Hz timer; `step` is a fraction of the remaining distance).
pub fn approach(current: f32, target: f32, step: f32) -> f32 {
    let step = step.clamp(0.0, 1.0);
    let delta = target - current;
    if delta.abs() <= 0.001 { return target; }
    current + delta * step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bell_is_one_under_the_pointer_and_zero_outside() {
        assert_eq!(bell(0.0), 1.0);
        assert_eq!(bell(1.0), 0.0);
        assert_eq!(bell(2.0), 0.0);
        assert_eq!(bell(f32::NAN), 0.0);
        assert!((bell(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bell_falls_off_monotonically() {
        let mut previous = bell(0.0);
        for step in 1..=20 {
            let value = bell(step as f32 / 20.0);
            assert!(value <= previous + 1e-6, "{value} after {previous}");
            previous = value;
        }
    }

    #[test]
    fn scale_stays_inside_configured_bounds() {
        assert!((scale(0.0, 100.0, 1.8) - 1.8).abs() < 1e-6);
        assert!((scale(1000.0, 100.0, 1.8) - 1.0).abs() < 1e-6);
        assert_eq!(scale(0.0, 100.0, 1.0), 1.0);
        // A corrupt maximum is clamped instead of reaching the layout.
        assert!(scale(0.0, 100.0, 9.0) <= 3.0);
        assert!(scale(0.0, 0.0, 1.8) <= 1.8);
    }

    #[test]
    fn approach_converges_without_overshooting() {
        let mut value = 1.0;
        for _ in 0..200 { value = approach(value, 1.5, 0.2); }
        assert!((value - 1.5).abs() < 1e-3);
        assert!((approach(2.0, 1.0, 0.5) - 1.5).abs() < 1e-6);
        assert_eq!(approach(1.0, 1.0, 0.5), 1.0);
    }
}
