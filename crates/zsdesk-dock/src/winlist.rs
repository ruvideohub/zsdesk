//! Top-level window enumeration and the window actions the dock offers.
//!
//! Only the facts needed by `model::is_application_window` are collected, and
//! the decision itself stays in the pure model module.

use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, LPARAM},
    Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
    System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION},
    UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindow,
        GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, GWL_EXSTYLE,
        GW_OWNER, IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_MINIMIZE,
        SW_RESTORE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    },
};

use windows::core::BOOL;

use crate::model::{is_application_window, WinInfo, WindowFacts};

struct Collect {
    foreground: HWND,
    windows: Vec<WinInfo>,
}

/// Every application window that should own a dock icon.
pub fn snapshot() -> Vec<WinInfo> {
    let mut collect = Collect { foreground: unsafe { GetForegroundWindow() }, windows: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(each_window), LPARAM((&mut collect as *mut Collect) as isize));
    }
    collect.windows
}

unsafe extern "system" fn each_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let collect = unsafe { &mut *(data.0 as *mut Collect) };
    if let Some(info) = describe(hwnd, collect.foreground) { collect.windows.push(info); }
    true.into()
}

fn describe(hwnd: HWND, foreground: HWND) -> Option<WinInfo> {
    let mut process = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    let class = class_name(hwnd);
    let title = window_text(hwnd);
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let owner = unsafe { GetWindow(hwnd, GW_OWNER) }.map(|owner| !owner.0.is_null()).unwrap_or(false);
    let facts = WindowFacts {
        own_process: process == std::process::id(),
        visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
        tool_window: style & WS_EX_TOOLWINDOW.0 != 0,
        cloaked: cloaked(hwnd),
        app_window: style & WS_EX_APPWINDOW.0 != 0,
        has_title: !title.is_empty(),
        has_owner: owner,
    };
    if !is_application_window(facts, &class) { return None; }
    let exe = process_path(process)?;
    Some(WinInfo { hwnd: hwnd.0 as isize, exe, title, minimized: unsafe { IsIconic(hwnd) }.as_bool(),
        foreground: hwnd == foreground })
}

/// Windows that belong to a virtual desktop other than the active one are
/// "cloaked"; they must not appear as running applications.
fn cloaked(hwnd: HWND) -> bool {
    let mut value = 0u32;
    unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&mut value as *mut u32).cast(), 4).is_ok() && value != 0
    }
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..length.min(buffer.len())])
}

fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    if length == 0 { return String::new(); }
    let mut buffer = vec![0u16; length.min(1024) + 1];
    let written = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..written.min(buffer.len())]).trim().to_string()
}

/// Full path of the executable behind a process id.
pub fn process_path(process: u32) -> Option<String> {
    if process == 0 { return None; }
    let handle: HANDLE = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut size = buffer.len() as u32;
    let result = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32,
        windows::core::PWSTR(buffer.as_mut_ptr()), &mut size) };
    let _ = unsafe { CloseHandle(handle) };
    result.ok()?;
    let size = (size as usize).min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..size]).trim().to_string())
}

/// Brings a window to the front, restoring it first when minimized.
pub fn activate(hwnd: isize) {
    let hwnd = HWND(hwnd as *mut _);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() { return; }
    if unsafe { IsIconic(hwnd) }.as_bool() { let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) }; }
    if unsafe { SetForegroundWindow(hwnd) }.as_bool() { return; }
    // Windows refuses `SetForegroundWindow` for a process that has not received
    // the last input event. The dock just did (the click), but a focus-stealing
    // helper can still lose the race; attach to the foreground thread once.
    foreground_with_input(hwnd);
}

fn foreground_with_input(hwnd: HWND) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    let current = unsafe { GetCurrentThreadId() };
    let target_thread = unsafe { GetWindowThreadProcessId(hwnd, None) };
    if target_thread == 0 || target_thread == current { return; }
    let attached = unsafe { AttachThreadInput(target_thread, current, true) }.as_bool();
    let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    let _ = unsafe { BringWindowToTop(hwnd) };
    let _ = unsafe { SetForegroundWindow(hwnd) };
    if attached { let _ = unsafe { AttachThreadInput(target_thread, current, false) }; }
}

pub fn minimize(hwnd: isize) {
    let hwnd = HWND(hwnd as *mut _);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() { let _ = unsafe { ShowWindow(hwnd, SW_MINIMIZE) }; }
}

pub fn close(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    let hwnd = HWND(hwnd as *mut _);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, Default::default(), Default::default()) };
    }
}

/// `true` while any window of the group is the foreground window.
pub fn forwardmost(windows: &[WinInfo]) -> Option<isize> {
    windows.iter().find(|window| window.foreground && !window.minimized).map(|window| window.hwnd)
}
