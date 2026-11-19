//! Process entry: single instance, DPI awareness, COM, window classes and the
//! message loop.

use std::{ffi::OsString, path::PathBuf};

use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HINSTANCE},
        System::{
            LibraryLoader::GetModuleHandleW,
            Ole::OleInitialize,
            Threading::CreateMutexW,
        },
        UI::{
            HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
            WindowsAndMessaging::{
                DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage, MSG, WNDCLASSW,
            },
        },
    },
};

use crate::{dock, launcher, language, panel_window, paths};

/// Mutex name; the dock is a single-instance component like the desktop host.
const MUTEX: &str = "Local\\ZSDesk.Dock";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// Profile folder passed by the file manager; `None` resolves the default.
    pub profile: Option<PathBuf>,
    pub help: bool,
}

impl Args {
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut args = Args::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.to_string_lossy().as_ref() {
                "--profile-dir" => {
                    let value = arguments.next().ok_or_else(|| "--profile-dir requires a folder".to_string())?;
                    let path = PathBuf::from(value);
                    if !path.is_absolute() { return Err("--profile-dir must be an absolute path".into()); }
                    args.profile = Some(path);
                }
                "--help" | "-h" | "/?" => args.help = true,
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        Ok(args)
    }
}

struct Instance(HANDLE);

impl Instance {
    /// `None` when another dock already owns the mutex.
    fn acquire(name: &str) -> windows::core::Result<Option<Self>> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) }?;
        // `CreateMutexW` succeeds for an existing name; the last error says so.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Ok(None);
        }
        Ok(Some(Self(handle)))
    }
}

impl Drop for Instance {
    fn drop(&mut self) { let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) }; }
}

/// Runs the dock until its window is destroyed.
pub fn run() -> Result<(), String> {
    let args = Args::parse(std::env::args_os().skip(1))?;
    if args.help {
        println!("zsdesk-dock {}", env!("CARGO_PKG_VERSION"));
        println!("  --profile-dir <folder>  use this ZSDesk profile");
        return Ok(());
    }
    language::initialize();
    let config_path = paths::config_path(args.profile);
    let _com = unsafe { OleInitialize(None) }.map_err(|error| error.message().to_string())?;
    unsafe { let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); }
    let instance = Instance::acquire(MUTEX).map_err(|error| error.message().to_string())?;
    if instance.is_none() {
        return Err(zsdesk_core::i18n::text("码头已在运行").to_string());
    }
    let instance = instance.unwrap();

    register(dock::CLASS, Some(dock::window_proc))?;
    register(launcher::CLASS, Some(launcher::window_proc))?;
    register(panel_window::CLASS, Some(panel_window::window_proc))?;
    dock::create(config_path).map_err(|error| error.message().to_string())?;

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 == 0 || result.0 == -1 { break; }
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }
    dock::destroy();
    drop(instance);
    Ok(())
}

fn register(class: &str, procedure: windows::Win32::UI::WindowsAndMessaging::WNDPROC) -> Result<(), String> {
    let instance = unsafe { GetModuleHandleW(None) }.map_err(|error| error.message().to_string())?;
    let wide: Vec<u16> = class.encode_utf16().chain(Some(0)).collect();
    let atom = unsafe {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: procedure,
            hInstance: HINSTANCE(instance.0),
            lpszClassName: PCWSTR(wide.as_ptr()),
            ..Default::default()
        })
    };
    if atom == 0 {
        Err(format!("RegisterClassW({class}): {}", unsafe { GetLastError() }.0))
    } else {
        Ok(())
    }
}

