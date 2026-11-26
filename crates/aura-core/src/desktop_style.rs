//! Shared desktop-surface design tokens, independent of any windowing backend.
//! Applications may override opacity, tint and corner radius per surface.
pub const SURFACE_DARK: [u8; 3] = [32, 32, 40];
pub const SURFACE_LIGHT: [u8; 3] = [246, 246, 250];
pub const CORNER_RADIUS_DIP: u8 = 24;
/// Corner radius of an interactive item or compact inset panel.
pub const ITEM_RADIUS_DIP: u8 = 8;

/// Settings groups use the Launchpad row geometry and a quiet ink overlay.
pub const SETTINGS_ROW_DIP: i32 = 56;
pub const SETTINGS_GROUP_RADIUS_DIP: i32 = 14;
pub const SETTINGS_GROUP_INK: f32 = 0.06;

/// Flatten the same ink overlay onto an opaque settings surface (native controls).
pub fn settings_group_surface(dark: bool) -> [u8; 3] {
    let base = if dark { SURFACE_DARK } else { SURFACE_LIGHT };
    let ink: [u8; 3] = if dark { [255, 255, 255] } else { [20, 20, 30] };
    // `ui::theme::ink` uses a reduced opacity in the light theme.
    let a = if dark { SETTINGS_GROUP_INK } else { SETTINGS_GROUP_INK * 0.6 };
    std::array::from_fn(|i| (f32::from(base[i]) * (1.0 - a) + f32::from(ink[i]) * a).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_groups_subtly_contrast_with_both_surfaces() {
        let dark = settings_group_surface(true);
        let light = settings_group_surface(false);
        assert!(dark.iter().zip(SURFACE_DARK).all(|(a,b)| *a > b));
        assert!(light.iter().zip(SURFACE_LIGHT).all(|(a,b)| *a < b));
        assert_eq!(SETTINGS_ROW_DIP, 56);
    }
}
