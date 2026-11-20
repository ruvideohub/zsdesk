//! `auraservice --run`: the supervisor.
//!
//! Every two seconds it reads the settings and makes reality match them: switched-on components
//! are started and kept started, and the ones this service started are stopped when their switch
//! goes off. A component that dies is started again, but no oftener than once a minute — a broken
//! component must not become a relaunch loop (the lesson of the window-hook guard).
//!
//! A component started by hand (the user opened AuraExplorer, an old logon entry started the
//! dock) is left alone: the service only manages what it started itself, so it can never close
//! a window the user opened on purpose.
//!
//! It is the only writer of the logon entry, and it takes a single-instance lock, so a second
//! copy started by hand (or by an old autostart entry) exits instead of starting the
//! components a second time.

use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};

use crate::launch::{self, log, Component, Running, Settings, COMPONENTS};

/// How often the settings are re-read and the children checked.
const TICK: Duration = Duration::from_secs(2);
/// How long a component that just died is left alone.
const BACKOFF: Duration = Duration::from_secs(60);
const MUTEX: windows::core::PCWSTR = w!("Local\\AuraExplorer.Service");

/// The supervisor's main loop; returns when there is nothing left to keep alive.
pub fn run() -> windows::core::Result<()> {
    unsafe {
        let Ok(lock) = CreateMutexW(None, false, MUTEX) else { return Ok(()) };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // A supervisor is already running: this copy must not start anything.
            let _ = windows::Win32::Foundation::CloseHandle(lock);
            return Ok(());
        }
        // Held for the life of the process; nothing to release by hand.
        std::mem::forget(lock);
    }
    log(&format!("service: started (pid {})", std::process::id()));

    let mut settings = launch::load();
    // The logon entry is ours to keep true, not just to write at install time.
    let _ = launch::logon::set(settings.autostart);

    let mut running: Vec<Running> = Vec::new();
    loop {
        let fresh = launch::load();
        if fresh.version != settings.version || fresh.autostart != settings.autostart || fresh.run != settings.run {
            if fresh.autostart != settings.autostart {
                let _ = launch::logon::set(fresh.autostart);
                log(&format!("service: autostart {}", if fresh.autostart { "on" } else { "off" }));
            }
            settings = fresh;
        }
        reconcile(&settings, &mut running);
        std::thread::sleep(TICK);
    }
}

/// Start what should run, stop what should not, and restart what died.
fn reconcile(settings: &Settings, running: &mut Vec<Running>) {
    // Forget children that are gone, and stop the ones that were switched off.
    let mut i = 0;
    while i < running.len() {
        let id = running[i].id;
        let wanted = COMPONENTS.iter().find(|c| c.id == id).map(|c| settings.wants(c)).unwrap_or(false);
        if !wanted {
            let mut done = running.remove(i);
            let _ = done.child.kill();
            let _ = done.child.wait();
            log(&format!("service: {id} stopped (switched off)"));
            continue;
        }
        if running[i].exited() {
            let mut done = running.remove(i);
            let _ = done.child.wait();
            // Back off, so a component that fails at once is not restarted in a loop.
            running.push(Running { id: done.id, child: done.child, retry_at: Instant::now() + BACKOFF });
            log(&format!("service: {id} exited; next attempt in {} s", BACKOFF.as_secs()));
            continue;
        }
        i += 1;
    }

    for component in COMPONENTS {
        if !settings.wants(component) {
            continue;
        }
        if running.iter().any(|r| r.id == component.id) {
            let entry = running.iter_mut().find(|r| r.id == component.id).expect("listed above");
            // A child that is still listed but already dead waits for its retry moment.
            if !entry.exited() {
                continue;
            }
            if Instant::now() < entry.retry_at {
                continue;
            }
            // Its retry moment has come: drop the old one and start a new process.
            running.retain(|r| r.id != component.id);
        }
        if launch::running(component.exe) {
            // Started outside the service (by the panel, by an old logon entry, by the user):
            // leave it be. It is not ours to stop or restart.
            continue;
        }
        match spawn(component, settings.profile_dir().as_deref()) {
            Some(child) => {
                log(&format!("service: {} started (pid {})", component.id, child.id()));
                running.push(Running { id: component.id, child, retry_at: Instant::now() });
            }
            None => log(&format!("service: {} could not start ({})", component.id, component.exe)),
        }
    }
}

fn spawn(component: &Component, profile: Option<&std::path::Path>) -> Option<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = launch::exe_of(component)?;
    let mut command = std::process::Command::new(exe);
    command.args(component.args).creation_flags(CREATE_NO_WINDOW);
    if let Some(profile) = profile.filter(|_| component.takes_profile) {
        // One profile for the whole suite: the service decides, the components receive it.
        command.arg("--profile-dir").arg(profile);
    }
    // Never leave a component working inside the install folder: an open handle there blocks
    // an update or an uninstall.
    if let Some(home) = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).filter(|p| p.is_dir()) {
        command.current_dir(home);
    }
    command.spawn().ok()
}

/// Is a supervisor running? (The settings window says whether the components are watched.)
pub fn active() -> bool {
    unsafe {
        match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, MUTEX) {
            Ok(handle) => {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
                true
            }
            Err(_) => false,
        }
    }
}
