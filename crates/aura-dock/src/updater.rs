//! Updates belong to AuraExplorer.
//!
//! Upstream Laminar Dock updates itself through Velopack, because it ships alone.
//! AuraExplorer ships the dock inside its own installer and updates it together with the
//! file manager, so this module keeps the updater's interface — the dock menu, the
//! Settings "About" rows and the timers still ask it things — and answers honestly:
//! this copy is installed by AuraExplorer, not by Setup.exe, so it never updates itself.
//!
//! Dropping Velopack also keeps about ninety crates (a TLS stack, an HTTP client,
//! ZIP) out of the build of a panel that only needs to draw itself.

use std::sync::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Where releases are published.
pub const REPO: &str = "https://github.com/ruvideohub/aura";

/// What the last update check found.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Shipped inside AuraExplorer, so there is nothing here to update.
    NotInstalled,
    /// Not checked yet.
    Idle,
    Checking,
    UpToDate,
    /// This version is downloaded and ready to install.
    Ready(String),
    Failed(String),
}

static STATUS: Mutex<Status> = Mutex::new(Status::NotInstalled);

/// Nothing to do at startup: no installer hooks, no pending update to resume.
pub fn startup() {}

/// False: AuraExplorer's installer owns this copy. The dock menu and the About section
/// read this to say "updates come with AuraExplorer" instead of offering an install.
pub fn installed() -> bool {
    false
}

/// "0.2.9 (AuraExplorer build)".
pub fn version_label() -> String {
    format!("{} (AuraExplorer build)", env!("CARGO_PKG_VERSION"))
}

pub fn status() -> Status {
    STATUS.lock().map(|status| status.clone()).unwrap_or(Status::NotInstalled)
}

fn set_status(status: Status) {
    if let Ok(mut current) = STATUS.lock() {
        *current = status;
    }
}

/// "Check for updates" from the dock menu. AuraExplorer updates everything at once, so this
/// reports the truth and posts `msg` back; the menu then shows the About text.
pub fn check(hwnd: HWND, msg: u32) {
    set_status(Status::NotInstalled);
    unsafe {
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0));
    }
}

/// Whether an automatic check is due. `hours` 0 means "only once per launch", and a
/// clock that moved backwards triggers a check rather than a day-long wait.
pub fn check_due(enabled: bool, hours: u32, last: u64, now: u64, first: bool) -> bool {
    enabled
        && match hours {
            0 => first,
            h => last > now || now - last >= h as u64 * 3600,
        }
}

/// No half-installed version can exist here: AuraExplorer installs atomically.
pub fn show_pending(_hwnd: HWND, _msg: u32) {}

pub fn begin_install() -> Result<(), String> {
    Err("AuraExplorer updates the panel together with the app: run the AuraExplorer installer.".into())
}

/// Release notes page for a version.
pub fn notes_url(version: &str) -> String {
    format!("{REPO}/releases/tag/v{version}")
}

/// Starting with Windows is AuraExplorer's job: it starts at login and brings the panel up
/// with it, so the panel never writes a startup entry of its own.
pub fn set_start_at_login(_on: bool) {}

/// Reports back what is actually in place: nothing of the panel's own.
pub fn sync_start_at_login(_wanted: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn update_checks_follow_the_chosen_frequency() {
        use super::check_due;
        let day = 24 * 3600;
        let now = 1_800_000_000;
        // Daily: not again within a day (a restart doesn't check), yes after a day.
        assert!(!check_due(true, 24, now - 3600, now, true));
        assert!(check_due(true, 24, now - day, now, false));
        assert!(check_due(true, 24, 0, now, true), "never checked before");
        // Weekly.
        assert!(!check_due(true, 168, now - 6 * day, now, false));
        assert!(check_due(true, 168, now - 7 * day, now, false));
        // At startup: once per launch, whatever the time.
        assert!(check_due(true, 0, now, now, true));
        assert!(!check_due(true, 0, now - 30 * day, now, false));
        // Turned off: never.
        assert!(!check_due(false, 24, 0, now, true));
        // Clock moved back: check rather than wait for the old time.
        assert!(check_due(true, 24, now + day, now, false));
    }

    #[test]
    fn the_panel_never_claims_to_update_itself() {
        use super::{installed, status, Status};
        assert!(!installed());
        assert_eq!(status(), Status::NotInstalled);
        assert!(super::version_label().ends_with("(AuraExplorer build)"));
    }
}
