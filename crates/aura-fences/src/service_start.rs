//! Open the integrated AuraUI service settings when Fences is installed beside it.
//! Standalone Fences packages intentionally omit the service and use Fences-owned startup.

use std::{path::PathBuf, process::Command};

fn location() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let service = executable.with_file_name("auraservice.exe");
    service.is_file().then_some(service)
}

pub fn available() -> bool { location().is_some() }

pub fn open_settings() {
    let Some(service) = location() else {
        crate::runtime::trace("service-settings-unavailable");
        return;
    };
    let mut command = Command::new(&service);
    command.arg("--settings");
    prepare(&mut command);
    match command.spawn() {
        Ok(_) => crate::runtime::trace("service-settings"),
        Err(error) => crate::runtime::trace(&format!("service-settings-failed {}: {error}", service.display())),
    }
}

fn prepare(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from).filter(|path| path.is_dir()) {
        command.current_dir(home);
    }
}
