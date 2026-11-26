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

/// Read only the shared Dock theme flag; malformed files have no preference.
pub fn dock_match_windows_setting(raw: &[u8]) -> Option<bool> {
    let text = std::str::from_utf8(raw).ok()?.trim_start_matches('\u{feff}');
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let object = json.as_object()?;
    match object.get("match_windows_theme") {
        None => Some(true),
        Some(value) => value.as_bool(),
    }
}

/// Dock follows Windows when enabled; otherwise it stays dark. `None` is the
/// standalone/missing-profile fallback and follows the system, not an invented profile.
pub fn resolve_dock_dark(match_windows: Option<bool>, system_dark: bool, forced: Option<&str>) -> bool {
    match forced {
        Some("dark") => true,
        Some("light") => false,
        _ => match_windows.map_or(system_dark, |follow| !follow || system_dark),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dock_config_handles_defaults_bom_and_invalid_data() {
        assert_eq!(dock_match_windows_setting(b"{}"), Some(true));
        assert_eq!(dock_match_windows_setting(b"{\"match_windows_theme\":false}"), Some(false));
        assert_eq!(dock_match_windows_setting("\u{feff}{\"match_windows_theme\":true}".as_bytes()), Some(true));
        assert_eq!(dock_match_windows_setting(b"{\"match_windows_theme\":\"false\"}"), None);
        assert_eq!(dock_match_windows_setting(b"{"), None);
    }
    #[test]
    fn dock_theme_inherits_or_falls_back_without_changing_preferences() {
        assert!(!resolve_dock_dark(Some(true), false, None));
        assert!(resolve_dock_dark(Some(false), false, None));
        assert!(!resolve_dock_dark(None, false, None));
        assert!(resolve_dock_dark(None, true, None));
        assert!(resolve_dock_dark(Some(true), false, Some("dark")));
        assert!(!resolve_dock_dark(Some(false), true, Some("light")));
    }
    #[test]
    fn settings_groups_subtly_contrast_with_both_surfaces() {
        let dark = settings_group_surface(true);
        let light = settings_group_surface(false);
        assert!(dark.iter().zip(SURFACE_DARK).all(|(a,b)| *a > b));
        assert!(light.iter().zip(SURFACE_LIGHT).all(|(a,b)| *a < b));
        assert_eq!(SETTINGS_ROW_DIP, 56);
    }
}
