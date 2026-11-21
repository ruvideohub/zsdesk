//! `auraservice` — the one place that starts AuraUI's pieces.
//!
//! The file manager, Fences, and the dock are independent applications. In an integrated
//! AuraUI installation this service is the shared owner of the startup settings and supervises
//! enabled components; a standalone Fences installation can still run without this service:
//!
//! * `auraservice` — the settings window: a card per component, one switch each, and the logon
//!   switch. What it shows is the truth on disk (`launch.json`) and in the process list.
//! * `auraservice --run` — the supervisor: keeps the switched-on components running and starts
//!   them at logon. Started by the window, by the logon entry, or by the installer.
//! * `auraservice --settings` — the settings window, explicitly (what the window's shortcut runs).
//! * `auraservice --status` — one line per component on stdout, for scripts and for the tests.

#![windows_subsystem = "windows"]

mod launch;
mod supervise;
mod window;

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next().unwrap_or_default();
    let result = match first.as_str() {
        "--run" => supervise::run(),
        "--settings" | "" => window::run(),
        "--status" => {
            reach_console();
            print_status();
            Ok(())
        }
        "--help" | "-h" | "/?" => {
            reach_console();
            print_help();
            Ok(())
        }
        other => {
            launch::log(&format!("unknown argument: {other}"));
            eprintln!("auraservice: неизвестный аргумент {other}");
            eprintln!("auraservice --help — список аргументов");
            std::process::exit(2);
        }
    };
    if let Err(error) = result {
        launch::log(&format!("fatal: {error}"));
        std::process::exit(1);
    }
}

/// The binary is a GUI application, so it never flashes a console when Windows starts it at
/// logon. When it *is* started from a terminal, borrow that terminal so `--status` and `--help`
/// can print into it.
fn reach_console() {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{AttachConsole, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
    }
    let Ok(console) = std::fs::OpenOptions::new().write(true).open("CONOUT$") else { return };
    let handle = HANDLE(console.as_raw_handle() as *mut std::ffi::c_void);
    unsafe {
        let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
        let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
    }
    // The console handle must outlive the prints.
    std::mem::forget(console);
}

fn print_help() {
    println!("auraservice — запуск компонентов AuraUI");
    println!();
    println!("  auraservice             окно настроек запуска");
    println!("  auraservice --settings  то же окно (для ярлыка)");
    println!("  auraservice --run       служба: держит включённые компоненты запущенными");
    println!("  auraservice --status    состояние компонентов, по строке на компонент");
    println!();
    println!();
    println!("Настройки: {}", launch::settings_path().display());
}

fn print_status() {
    let settings = launch::load();
    for component in launch::COMPONENTS {
        let installed = launch::exe_of(component).is_some();
        let alive = installed && launch::running(component.exe);
        println!(
            "{}\t{}\t{}\t{}",
            component.id,
            if settings.wants(component) { "on" } else { "off" },
            if alive { "running" } else { "stopped" },
            if installed { "installed" } else { "missing" },
        );
    }
    println!("# service\t{}", if supervise::active() { "running" } else { "stopped" });
    println!("# autostart\t{}", if settings.autostart { "on" } else { "off" });
    println!("# settings\t{}", launch::settings_path().display());
}
