//! The AuraUI service, started by the file manager when nobody else has.
//!
//! Every piece of the suite is its own executable now (the panel, the fences, the widgets), and
//! one service starts them: `auraservice.exe`. The file manager does not start any of them —
//! that is what keeps a single owner of the run settings — but a normal launch makes sure the
//! service itself is up, so launching AuraExplorer from a folder (not from the Start menu)
//! still gives a complete desktop.
//!
//! `auraservice --run` takes a single-instance lock: when a supervisor is already running, the
//! copy started here exits at once and starts nothing a second time.

use std::path::PathBuf;
use std::process::Command;

/// Should this launch make sure the service is running?
///
/// Yes for a normal launch of an edition with the desktop, no for the helper modes that only
/// talk to a running host, and no when the user asked for `--no-dock` (a plain file window, or
/// a session where the panel is in the way).
pub fn wanted(args: &[std::ffi::OsString]) -> bool {
    if !crate::edition::DESKTOP {
        return false;
    }
    let flag = |name: &str| args.iter().any(|argument| argument == name);
    let quiet = flag("--no-dock")
        || flag("--quick-search")
        || flag("--quick-search-settings")
        || flag("--background");
    // `--desktop --quit` asks the running fences to go away and starts nothing, so a
    // request to quit must never bring the desktop up on its way out.
    let quitting = flag("--quit") && flag("--desktop");
    !quiet && !quitting
}

/// Start `auraservice.exe --run` if it ships next to this executable. Failure is logged and
/// ignored: a missing or broken service must never stop the file manager.
pub fn ensure_running() {
    let Some(service) = location() else {
        crate::runtime::trace("service-missing");
        return;
    };
    match spawn(&service) {
        Some(_) => crate::runtime::trace(&format!("service-start {}", service.display())),
        None => crate::runtime::trace(&format!("service-start-failed {}", service.display())),
    }
}

/// `auraservice.exe` beside `AuraExplorer.exe`, as the installer lays it out.
fn location() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let beside = executable.with_file_name("auraservice.exe");
    beside.is_file().then_some(beside)
}

fn spawn(service: &std::path::Path) -> Option<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(service);
    command.arg("--run").creation_flags(CREATE_NO_WINDOW);
    // Every app the service opens inherits this folder. Leaving it pointing into the install
    // folder keeps a handle open there and breaks an update or uninstall with "file in use".
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from).filter(|path| path.is_dir()) {
        command.current_dir(home);
    }
    command.spawn().ok()
}

#[cfg(test)]
mod tests {
    use super::wanted;
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_plain_launch_brings_the_service() {
        assert!(wanted(&args(&["--desktop"])));
        assert!(wanted(&args(&[])));
    }

    #[test]
    fn helper_modes_leave_the_service_alone() {
        assert!(!wanted(&args(&["--no-dock"])));
        assert!(!wanted(&args(&["--quick-search"])));
        assert!(!wanted(&args(&["--quick-search-settings"])));
        assert!(!wanted(&args(&["--background"])));
    }

    #[test]
    fn closing_the_fences_does_not_start_anything() {
        assert!(!wanted(&args(&["--desktop", "--quit"])));
    }
}
