//! Laminar Dock as a library, shared by the two executables built from this package.
//!
//! - `AuraDock.exe` (`src/main.rs`) — the panel: icons, Launchpad, previews, the control
//!   centre, the tray.
//! - `widgets.exe` (`src/bin/widgets.rs`) — the desktop-widget host. Its first widget is the
//!   desktop clock, which used to be a window inside the panel's own process: a hang or a
//!   redraw problem on the desktop must not take the panel (and the taskbar it replaces)
//!   down with it. Widget faces are drawn by the same code in all three places — the panel,
//!   Launchpad and the desktop — which is why this is a library and not a copy.
//!
//! The panel stays the single writer of the settings file: `widgets.exe` only reads it and
//! reports changes by posting the messages the card has always posted, now across processes.
#![cfg_attr(not(debug_assertions), allow(dead_code))]

mod appbar_policy;
mod apps;
mod config;
mod control_center;
mod desktop_clock;
mod desktop_retry;
mod desktop_system;
mod displays;
mod dock;
mod effects;
mod filedrop;
mod gfx;
mod host;
mod iconcache;
mod launchpad;
mod launchpad_input;
mod launch_target;
mod latest_worker;
mod launchpad_command_aliases;
mod menu;
mod metrics;
mod preview;
mod services;
mod stack;
mod startup;
mod startup_policy;
mod strings;
mod taskbar;
mod taskbar_guard;
mod tracker;
mod tray;
mod tray_policy;
mod tray_compact;
mod tray_mini;
mod tray_mini_layout;
mod tray_strip;
mod ui;
mod updater;
mod widgets;
mod winkey;

use windows::core::Result;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

/// Command-line options.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Measure startup, memory and CPU, print JSON, then exit.
    pub measure: bool,
    /// Hold the dock in its magnified state (for screenshots).
    pub demo_hover: bool,
    /// Quit automatically after this many seconds.
    pub exit_after: Option<u32>,
    /// Open a surface automatically for visual testing: menu, preview, stack, launchpad, settings.
    pub demo: Option<String>,
    /// Helper dock for an extra display (index into displays::all()).
    pub monitor: Option<usize>,
    /// The main dock's process id (helpers quit when it exits).
    pub parent: Option<u32>,
    /// `--settings`: open the panel's settings page. If a panel is already running, the page
    /// opens there — asking for settings must never do nothing at all.
    pub settings: bool,
}

/// Work from the user's folder, not the install folder Velopack starts us in. Every app
/// the dock opens inherits this working folder; one left pointing into
/// `LaminarDock\current` (Slack, say) stops Velopack renaming that folder, and the update
/// fails with "file in use" until the app quits.
fn leave_install_folder() {
    let home = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).filter(|p| p.is_dir());
    if let Some(dir) = home.or_else(|| std::env::var_os("SystemRoot").map(std::path::PathBuf::from)) {
        let _ = std::env::set_current_dir(dir);
    }
}

/// Ask Windows for the per-monitor DPI scale (the panel is laid out in logical px).
fn dpi_aware() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// The apartment and the dispatcher queue Windows.UI.Composition needs on the UI thread.
fn composition_apartment() -> Result<()> {
    unsafe {
        // OLE (COM single-threaded apartment + drag and drop support).
        windows::Win32::System::Ole::OleInitialize(None)?;
    }
    // Windows.UI.Composition needs a DispatcherQueue on the UI thread.
    let _queue = unsafe {
        CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        })?
    };
    // The controller lives as long as the thread's message loop: leak it on purpose.
    std::mem::forget(_queue);
    Ok(())
}

/// The panel: `AuraDock.exe` (and the helper docks on extra displays).
pub fn run() -> Result<()> {
    let started = std::time::Instant::now();
    // Velopack's install / update / uninstall steps run here and exit (see updater.rs).
    updater::startup();
    // The panel's own labels: Russian unless the settings say otherwise.
    if let Some(cfg) = config::Config::load() {
        strings::set_language(&cfg.language);
    }
    leave_install_folder();
    let args: Vec<String> = std::env::args().skip(1).collect();

    // `--component clock --dock <hwnd>`: the same binary standing in for `widgets.exe` when
    // that file is missing (an installation updated by copying just the panel).
    if let Some(component) = component_arg(&args) {
        return match component.as_str() {
            "clock" => host::run_clock(dock_arg(&args)),
            "system" => host::run_system(dock_arg(&args)),
            other => {
                eprintln!("unknown component: {other}");
                Ok(())
            }
        };
    }

    // `--watchdog <pid> [taskbar-state]`: tiny helper that restores the Windows taskbar
    // when the dock process exits, however it exits.
    if args.first().map(|a| a == "--watchdog").unwrap_or(false) {
        if let Some(pid) = args.get(1).and_then(|p| p.parse().ok()) {
            taskbar::run_watchdog(pid, args.get(2).and_then(|s| s.parse().ok()));
        }
        return Ok(());
    }

    // `--list-windows`: print what the dock sees (app identity, exe, title) for troubleshooting.
    if args.first().map(|a| a == "--list-windows").unwrap_or(false) {
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        for w in tracker::Tracker::default().scan() {
            println!("{:<70} | {:<60} | {}", w.app, w.exe, w.title);
        }
        return Ok(());
    }

    // `--test-open-with <app id> <file>`: diagnostics for opening files with a dock app.
    if args.first().map(|a| a == "--test-open-with").unwrap_or(false) {
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
        if let (Some(app), Some(file)) = (args.get(1), args.get(2)) {
            filedrop::open_with(app, None, &[file.clone()]);
            std::thread::sleep(std::time::Duration::from_secs(4));
        }
        return Ok(());
    }

    // `--widget-sheet out.png [light] [scale=2] [empty] [filter]`: render every widget at every size to a PNG.
    if args.first().map(|a| a == "--widget-sheet").unwrap_or(false) {
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
        let out = args.get(1).cloned().unwrap_or_else(|| "widgets.png".into());
        let flag = |a: &str| a == "light" || a == "empty" || a.starts_with("scale=");
        let light = args.iter().any(|a| a == "light");
        let empty = args.iter().any(|a| a == "empty");
        let scale = args.iter().find_map(|a| a.strip_prefix("scale=")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0).clamp(1.0, 3.0);
        let filter = args.iter().skip(2).find(|a| !flag(a)).cloned();
        if let Err(e) = widgets::sheet::run(&out, light, filter.as_deref(), scale, empty) {
            eprintln!("widget sheet failed: {e:?}");
        }
        return Ok(());
    }

    let mut opts = Options::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--measure" => opts.measure = true,
            "--demo-hover" => opts.demo_hover = true,
            "--exit-after" => opts.exit_after = it.next().and_then(|v| v.parse().ok()),
            "--demo" => opts.demo = it.next().cloned(),
            "--monitor" => opts.monitor = it.next().and_then(|v| v.parse().ok()),
            "--parent" => opts.parent = it.next().and_then(|v| v.parse().ok()),
            "--settings" => opts.settings = true,
            _ => {}
        }
    }

    unsafe {
        // Release builds have no console; reuse the parent's so --measure can print.
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        // One real dock at a time (test modes may run alongside).
        if !(opts.measure || opts.demo_hover || opts.demo.is_some()) {
            let name = match opts.monitor {
                Some(i) => format!("Local\\AuraExplorer.Dock-{i}"),
                None => "Local\\AuraExplorer.Dock".to_string(),
            };
            let _mutex = CreateMutexW(None, false, &windows::core::HSTRING::from(name));
            if GetLastError() == ERROR_ALREADY_EXISTS {
                // One panel per display: a second copy leaves at once. `--settings` still has
                // to arrive somewhere, so it is handed to the panel that is already running.
                if opts.settings {
                    dock::show_settings_of_running_panel();
                }
                return Ok(());
            }
            std::mem::forget(_mutex);
        }
    }
    dpi_aware();
    composition_apartment()?;
    dock::run(opts, started)
}

/// The desktop-widget host: `widgets.exe`.
///
/// It hosts the desktop clock card (`--component clock`, the default) and the desktop load
/// card (`--component system`). Each takes its own single-instance lock, so a second copy of
/// one card exits quietly instead of drawing it twice, and a card quits when the panel that
/// started it is gone.
pub fn run_widgets() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::var_os("USERPROFILE").is_some() {
        leave_install_folder();
    }
    let mut dock = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--dock" {
            dock = it.next().and_then(|v| parse_hwnd(v));
        }
    }
    // One card per process, and one lock per card: the clock and the load card are two
    // processes of this same program, so a lock shared by both would keep the second card
    // from ever appearing.
    let component = component_arg(&args).unwrap_or_else(|| "clock".to_string());
    unsafe {
        let name = windows::core::HSTRING::from(format!("Local\\AuraExplorer.Widgets.{component}"));
        let mutex = CreateMutexW(None, false, &name);
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Another widgets.exe is already drawing this card: nothing to do.
            return Ok(());
        }
        std::mem::forget(mutex);
    }
    match component.as_str() {
        "system" => host::run_system(dock),
        _ => host::run_clock(dock),
    }
}

/// `--component <name>` from the command line.
fn component_arg(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--component")?;
    args.get(i + 1).cloned()
}

/// `--dock <0x...>`: the panel's window, as hexadecimal (it is a pointer).
fn dock_arg(args: &[String]) -> Option<windows::Win32::Foundation::HWND> {
    let i = args.iter().position(|a| a == "--dock")?;
    args.get(i + 1).and_then(|v| parse_hwnd(v))
}

fn parse_hwnd(v: &str) -> Option<windows::Win32::Foundation::HWND> {
    let digits = v.trim_start_matches("0x").trim_start_matches("0X");
    let raw = isize::from_str_radix(digits, 16).ok()?;
    (raw != 0).then_some(windows::Win32::Foundation::HWND(raw as _))
}
