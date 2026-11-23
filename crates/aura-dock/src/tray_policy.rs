//! Decisions shared by the Shell tray host and its layout; no Windows dependencies.
#[derive(Debug, PartialEq, Eq)]
pub enum Placement {
    HiddenByApp,
    Strip,
    Overflow,
}

pub fn placement(app_hidden: bool, user_hidden: bool, strip_has_room: bool) -> Placement {
    if app_hidden {
        Placement::HiddenByApp
    } else if user_hidden || !strip_has_room {
        Placement::Overflow
    } else {
        Placement::Strip
    }
}

/// On an add by GUID the application may have restarted with a new callback ID.
/// A GUID-only modify does not necessarily supply a meaningful uID.
pub fn registration_uid(is_add: bool, has_guid: bool, current: u32, incoming: u32) -> u32 {
    if is_add && has_guid {
        incoming
    } else {
        current
    }
}

/// A cached HWND identity is usable only while it belongs to the same process.
pub fn reuse_identity(cached_pid: u32, current_pid: u32, app: &str) -> bool {
    current_pid != 0 && cached_pid == current_pid && !app.is_empty()
}

/// The helper/watchdog must not mistake a Dock-owned tray proxy for Explorer.
pub fn is_dock_image(candidate: &str, current: &str) -> bool {
    if candidate.is_empty() {
        return false;
    }
    if !current.is_empty() && candidate.eq_ignore_ascii_case(current) {
        return true;
    }
    let name = candidate.rsplit(['\\', '/']).next().unwrap_or(candidate);
    name.eq_ignore_ascii_case("AuraDock.exe") || name.eq_ignore_ascii_case("Laminar.exe")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_hidden_icons_never_reappear_in_overflow() {
        for user_hidden in [false, true] {
            for room in [false, true] {
                assert_eq!(placement(true, user_hidden, room), Placement::HiddenByApp);
            }
        }
        assert_eq!(placement(false, true, true), Placement::Overflow);
        assert_eq!(placement(false, false, false), Placement::Overflow);
        assert_eq!(placement(false, false, true), Placement::Strip);
    }
    #[test]
    fn guid_reregistration_updates_the_callback_id_but_modify_does_not() {
        assert_eq!(registration_uid(true, true, 1, 7), 7);
        assert_eq!(registration_uid(false, true, 7, 0), 7);
        assert_eq!(registration_uid(true, false, 7, 0), 7);
    }
    #[test]
    fn stale_or_unresolved_window_identity_is_not_reused() {
        assert!(reuse_identity(42, 42, "app"));
        assert!(!reuse_identity(42, 77, "app"));
        assert!(!reuse_identity(0, 0, "app"));
        assert!(!reuse_identity(42, 42, ""));
    }
    #[test]
    fn dock_images_are_distinct_from_explorer() {
        assert!(is_dock_image(
            r"C:\Apps\AuraDock.exe",
            r"C:\Apps\AuraDock.exe"
        ));
        assert!(is_dock_image(
            r"C:\Apps\AURADOCK.EXE",
            r"C:\Other\helper.exe"
        ));
        assert!(is_dock_image(
            r"C:\Apps\renamed.exe",
            r"c:\apps\Renamed.exe"
        ));
        assert!(is_dock_image(r"C:\Apps\laminar.exe", ""));
        assert!(!is_dock_image(
            r"C:\Windows\explorer.exe",
            r"C:\Apps\AuraDock.exe"
        ));
        assert!(!is_dock_image("", ""));
    }
}
