//! AuraExplorer as a library: the file window, the desktop fence host and the small
//! background programs, shared by the two executables built from this package.
//!
//! - `AuraExplorer.exe` (`src/main.rs`) — the file manager, every background mode, and the
//!   desktop host when an older installation has no `fences.exe`.
//! - `fences.exe` (`src/bin/fences.rs`) — the desktop fence host on its own: it is what the
//!   file window starts for the desktop, so a crash or a hang there no longer takes the file
//!   window with it, and the two can be updated separately.

mod app;
mod language;
mod commands;
// Compiled in every edition; unreachable without the `desktop` feature and dropped by the linker.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod desktop;
mod edition;
mod storage;
mod window;
mod instance;
mod reveal;
mod shell_dock;
mod shell_window_registry;
mod context_preferences;
mod flyout_menu;
mod routing_control;
mod fast_search;
mod folder_size_data;
mod quick_search;
mod win_e_bridge;
mod explorer_forwarder;
mod early_window_hook;
mod file_dialog_takeover;
mod folder_dde;
mod runtime;
mod explorer_tray;
mod update_shutdown;
mod context_menu_process;
mod utility_panel;
mod update_check;

#[cfg(test)]
mod icon_worker_tests{
    #[test]
    #[ignore="Owned hidden icon-worker endpoint for desktop fixtures"]
    fn native_icon_worker_entry(){aura_windows::shell::icons::run_icon_test_worker().unwrap();}
}

/// Lets the installer wait for all application windows and desktop guards to
/// finish before replacing or uninstalling their files.
struct ApplicationLifetime(Option<windows::Win32::Foundation::HANDLE>);
impl ApplicationLifetime {
    fn new() -> Self {
        if app::is_import_cli() { return Self(None); }
        use windows::Win32::System::Threading::{CreateMutexExW, SYNCHRONIZATION_SYNCHRONIZE};
        let name: Vec<u16> = crate::edition::application_mutex().encode_utf16().chain(Some(0)).collect();
        Self(unsafe { CreateMutexExW(None, windows::core::PCWSTR(name.as_ptr()), 0, SYNCHRONIZATION_SYNCHRONIZE.0) }.ok())
    }
}
impl Drop for ApplicationLifetime {
    fn drop(&mut self) { if let Some(handle) = self.0.take() { let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) }; } }
}

/// Set by `fences.exe` before the arguments are parsed: that program is the desktop host,
/// and with no mode of its own it shows the fences instead of a file window.
static FENCES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Is this the `fences.exe` program?
pub(crate) fn fences_component() -> bool {
    FENCES.load(std::sync::atomic::Ordering::Relaxed)
}

/// `AuraExplorer.exe`: every mode, chosen by the command line.
pub fn run() {
    language::initialize_environment();
    if let Some(result)=update_shutdown::run_if_requested(){if let Err(error)=result{eprintln!("{error:#}");std::process::exit(1);}return;}
    if let Some(result)=aura_windows::shell::icons::maybe_handle_icon_worker(&std::env::args_os().skip(1).collect::<Vec<_>>()){
        if let Err(error)=result{eprintln!("{error}");std::process::exit(1);}return;
    }
    if let Some(result)=file_dialog_takeover::guard::run_if_requested(){if let Err(error)=result{eprintln!("{error:#}");std::process::exit(1);}return;}
    if let Some(result)=aura_windows::shell::maybe_handle_packaged_state_worker(&std::env::args_os().skip(1).collect::<Vec<_>>()){
        if let Err(error)=result{eprintln!("{error}");std::process::exit(1);}return;
    }
    if let Some(result)=explorer_forwarder::run_guard_if_requested(){if let Err(error)=result{eprintln!("{error:#}");std::process::exit(1);}return;}
    if let Some(result)=context_menu_process::run_worker_if_requested(){if let Err(error)=result{eprintln!("{error:#}");std::process::exit(1);}return;}
    if let Some(result)=window::preview_process::run_if_requested(){
        if let Err(error)=result{eprintln!("{error:#}");std::process::exit(1);}return;
    }
    let _lifetime = ApplicationLifetime::new();
    if let Err(error)=app::run(){
        if app::is_import_cli() || app::is_background_mode() {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        // Folders opened through system integration must never lose their opener.
        if let Some(target)=app::shell_open_target() {
            let _=app::open_original_explorer(target);
            return;
        }
        if app::is_win_e() && app::open_original_explorer("shell:MyComputerFolder".into()).is_ok(){return;}
        let message:Vec<u16>=format!("{error:#}").encode_utf16().chain(Some(0)).collect();
        unsafe {windows::Win32::UI::WindowsAndMessaging::MessageBoxW(None,windows::core::PCWSTR(message.as_ptr()),
            windows::core::w!("AuraExplorer"),windows::Win32::UI::WindowsAndMessaging::MB_OK|windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR);}
        std::process::exit(1);
    }
}

/// `fences.exe`: the desktop fence host, as its own program.
pub fn run_fences() {
    FENCES.store(true, std::sync::atomic::Ordering::Relaxed);
    run();
}
