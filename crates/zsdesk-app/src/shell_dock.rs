//! The ZSDesk panel, started together with the desktop.
//!
//! The panel (the dock, its tray and the Launchpad) is `zsdesk-dock.exe`, the crate
//! `crates/zsdesk-dock` — Laminar Dock, MIT, vendored and trimmed. ZSDesk starts it
//! next to itself, so one install, one icon and one launch give the fences *and* the
//! panel.
//!
//! The panel is told our process id (`--parent`), and the dock closes itself when we
//! exit — including when we are killed. That also puts the Windows taskbar back,
//! because the dock restores it on the way out.

use std::path::PathBuf;
use std::process::{Child, Command};

/// Handle on the running panel. Dropping it stops the panel.
pub struct Panel {
    child: Option<Child>,
    executable: PathBuf,
}

impl Panel {
    /// Start the panel if it ships next to this executable. Failure is logged and
    /// ignored: a missing or broken panel must never stop the file manager.
    pub fn start() -> Self {
        let executable = location();
        let child = executable.as_deref().and_then(spawn);
        match (&child, &executable) {
            (Some(_), Some(_)) => crate::runtime::trace("dock-start"),
            (None, Some(path)) => crate::runtime::trace(&format!("dock-start-failed {}", path.display())),
            _ => crate::runtime::trace("dock-missing"),
        }
        Self { child, executable: executable.unwrap_or_default() }
    }

    /// Stop the panel now, so the taskbar is back before this process disappears.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            crate::runtime::trace(&format!("dock-stop {}", self.executable.display()));
        }
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `zsdesk-dock.exe` beside `ZSDesk.exe`, as the installer and the dock build lay it out.
fn location() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let dock = executable.with_file_name("zsdesk-dock.exe");
    dock.is_file().then_some(dock)
}

fn spawn(dock: &std::path::Path) -> Option<Child> {
    let mut command = Command::new(dock);
    command.arg("--parent").arg(std::process::id().to_string());
    // Every app the panel opens inherits this folder. Leaving it pointing into the
    // install folder keeps a handle open there and breaks an update or uninstall
    // with "file in use" until the app quits.
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from).filter(|path| path.is_dir()) {
        command.current_dir(home);
    }
    command.spawn().ok()
}

/// Should this launch show the panel?
///
/// Yes for a normal launch of an edition with the desktop, no for the helper modes
/// that only talk to a running host, and no when the user asked for `--no-dock`
/// (a plain file window, or a session where the panel is in the way).
pub fn wanted(args: &[std::ffi::OsString]) -> bool {
    if !crate::edition::DESKTOP {
        return false;
    }
    let flag = |name: &str| args.iter().any(|argument| argument == name);
    let quiet = flag("--no-dock")
        || flag("--quick-search")
        || flag("--quick-search-settings")
        || flag("--background");
    // `--desktop quit` asks the running fences to go away and starts nothing.
    let quitting = flag("quit") && flag("--desktop");
    !quiet && !quitting
}

#[cfg(test)]
mod tests {
    use super::wanted;
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_plain_launch_brings_the_panel() {
        assert!(wanted(&args(&["--desktop"])));
        assert!(wanted(&args(&[])));
    }

    #[test]
    fn helper_modes_leave_the_panel_alone() {
        assert!(!wanted(&args(&["--no-dock"])));
        assert!(!wanted(&args(&["--quick-search"])));
        assert!(!wanted(&args(&["--quick-search-settings"])));
        assert!(!wanted(&args(&["--background"])));
    }

    #[test]
    fn closing_the_fences_does_not_start_one() {
        assert!(!wanted(&args(&["--desktop", "quit"])));
    }
}
