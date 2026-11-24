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

/// A partial icon learned from MODIFY must be allowed to complete registration; a
/// GUID can also be claimed by a new owner after an application restart.
pub fn accepts_readd(provisional: bool, old_callback: u32, owner_changed: bool) -> bool {
    provisional || old_callback == 0 || owner_changed
}

/// Windows notification callback ABI: VERSION_4 uses screen coordinates in wParam
/// and a 16-bit identifier in lParam; older versions use the full 32-bit ID.
pub fn callback_words(version: u32, uid: u32, event: u32, x: i32, y: i32) -> (usize, isize) {
    if version >= 4 {
        let point = ((y as u16 as usize) << 16) | (x as u16 as usize);
        let encoded = ((uid as usize & 0xffff) << 16) | (event as usize & 0xffff);
        (point, encoded as isize)
    } else {
        (uid as usize, event as isize)
    }
}

/// A control-centre click activates only when press and release match.
pub fn matching_release(pressed: Option<u64>, released: Option<u64>) -> Option<u64> {
    pressed.filter(|key| released == Some(*key))
}

/// A double-click needs the same icon, the Windows time limit, and its click rectangle.
pub fn double_click(same_icon: bool, elapsed_ms: u128, limit_ms: u32, dx: i64, dy: i64, width: i32, height: i32) -> bool {
    same_icon && elapsed_ms <= limit_ms as u128
        && dx.abs() * 2 <= i64::from(width.max(1))
        && dy.abs() * 2 <= i64::from(height.max(1))
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
    fn callback_abi_preserves_legacy_ids_and_signed_monitor_coordinates() {
        assert_eq!(callback_words(0, 0x1234_5678, 0x205, -10, 900), (0x1234_5678, 0x205));
        let (wp, lp) = callback_words(4, 0x1234_5678, 0x7b, -10, -20);
        assert_eq!((wp & 0xffff) as u16 as i16, -10);
        assert_eq!(((wp >> 16) & 0xffff) as u16 as i16, -20);
        assert_eq!(lp as usize & 0xffff, 0x7b);
        assert_eq!((lp as usize >> 16) & 0xffff, 0x5678);
    }
    #[test]
    fn partial_or_new_owner_may_register_but_normal_duplicate_is_rejected() {
        assert!(!accepts_readd(false, 0x8050, false));
        assert!(accepts_readd(true, 0x8050, false)); // MODIFY included a callback before ADD
        assert!(accepts_readd(false, 0, false));
        assert!(accepts_readd(false, 0x8050, true));
    }
    #[test]
    fn click_requires_release_on_the_pressed_icon() {
        assert_eq!(matching_release(Some(12), Some(12)), Some(12));
        assert_eq!(matching_release(Some(12), Some(13)), None);
        assert_eq!(matching_release(Some(12), None), None);
        assert_eq!(matching_release(None, Some(12)), None);
    }
    #[test]
    fn double_click_requires_same_icon_and_windows_time_and_distance() {
        assert!(double_click(true, 250, 500, 1, -2, 4, 4));
        assert!(!double_click(false, 250, 500, 0, 0, 4, 4));
        assert!(!double_click(true, 501, 500, 0, 0, 4, 4));
        assert!(!double_click(true, 250, 500, 3, 0, 4, 4));
        assert!(!double_click(true, 250, 500, 0, -3, 4, 4));
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
