//! Ensure the AuraUI launch service is available for a normal file-manager launch.
//!
//! The service owns the integrated suite's startup settings and starts independent programs
//! such as `Fences.exe` and `AuraDock.exe`; AuraExplorer only ensures the service is running.
//!
//! `auraservice --run` takes a single-instance lock: when a supervisor is already running, the
//! copy started here exits at once and starts nothing a second time.

use std::path::PathBuf;
use std::process::Command;

/// Should this launch make sure the service is running?
///
/// Yes for a normal launch of an edition that ships the companion programs; no for helper
/// modes that only talk to a running host, or when `--no-dock` requests a plain file window
/// without starting the integrated companion set.
pub fn wanted(args: &[std::ffi::OsString]) -> bool {
    if !crate::edition::DESKTOP {
        return false;
    }
    let flag = |name: &str| args.iter().any(|argument| argument == name);
    let quiet = flag("--no-dock")
        || flag("--quick-search")
        || flag("--quick-search-settings")
        || flag("--background");
    !quiet
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

/// Open the service's launch settings window. The standalone Fences app has a matching
/// bridge so both applications use the same controls when the service is installed.
pub fn open_settings() {
    let Some(service) = location() else {
        crate::runtime::trace("service-missing");
        return;
    };
    let mut command = Command::new(&service);
    command.arg("--settings");
    prepare(&mut command);
    match command.spawn() {
        Ok(_) => crate::runtime::trace("service-settings"),
        Err(_) => crate::runtime::trace(&format!("service-settings-failed {}", service.display())),
    }
}

/// `auraservice.exe` beside `AuraExplorer.exe`, as the installer lays it out.
fn location() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let beside = executable.with_file_name("auraservice.exe");
    beside.is_file().then_some(beside)
}

fn spawn(service: &std::path::Path) -> Option<std::process::Child> {
    let mut command = Command::new(service);
    command.arg("--run");
    prepare(&mut command);
    command.spawn().ok()
}

fn prepare(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
    // Every app the service opens inherits this folder. Leaving it pointing into the install
    // folder keeps a handle open there and breaks an update or uninstall with "file in use".
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from).filter(|path| path.is_dir()) {
        command.current_dir(home);
    }
}

#[cfg(test)]
mod tests {
    use super::wanted;
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_plain_launch_starts_the_service_only_when_the_companion_is_included() {
        assert_eq!(wanted(&args(&[])), crate::edition::DESKTOP);
    }

    #[test]
    fn helper_modes_leave_the_service_alone() {
        assert!(!wanted(&args(&["--no-dock"])));
        assert!(!wanted(&args(&["--quick-search"])));
        assert!(!wanted(&args(&["--quick-search-settings"])));
        assert!(!wanted(&args(&["--background"])));
    }
}
