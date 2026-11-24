//! Taking over from the Windows taskbar, safely.
//!
//! * Hide Explorer's taskbar windows while Laminar runs (Explorer itself keeps
//!   running: desktop, File Explorer, Start and notifications still work).
//! * If the taskbar wasn't set to auto-hide, switch it to auto-hide so it stops
//!   reserving screen space, and remember to switch it back.
//! * A watchdog process waits for Laminar to exit for ANY reason (quit, crash,
//!   Task Manager) and then restores the taskbar.
//! * When the dock is always visible it registers as an "AppBar", so maximized
//!   windows stop above it, like they do above the Windows taskbar.

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABE_BOTTOM, ABE_TOP, ABM_GETSTATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_SETSTATE,
    ABS_AUTOHIDE, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, ShowWindow, SW_HIDE, SW_SHOWNA};

use crate::dock::WM_APP_APPBAR;

fn trays() -> Vec<HWND> {
    let mut v = Vec::new();
    // Explorer's own taskbar, never the main dock's tray stand-in (same class name).
    if let Some(h) = crate::tray::explorer_tray() {
        v.push(h);
    }
    unsafe {
        let mut prev: Option<HWND> = None;
        while let Ok(h) = FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), None) {
            v.push(h);
            prev = Some(h);
        }
    }
    v
}

// The panel used to ask here whether Explorer's taskbar was on screen and move to the top
// when it was. That probe is gone: an auto-hiding or briefly shown taskbar flipped the panel
// between edges. The edge now follows the "Hide the Windows Taskbar" checkbox alone.

pub fn set_visible(visible: bool) {
    for h in trays() {
        unsafe {
            let _ = ShowWindow(h, if visible { SW_SHOWNA } else { SW_HIDE });
        }
    }
}

fn abd(hwnd: Option<HWND>) -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd.unwrap_or_default(),
        ..Default::default()
    }
}

/// Explorer taskbar state bits (ABS_AUTOHIDE etc).
pub fn tray_state() -> u32 {
    let mut d = abd(trays().first().copied());
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) as u32 }
}

pub fn set_tray_state(state: u32) {
    let mut d = abd(trays().first().copied());
    d.lParam = LPARAM(state as isize);
    unsafe {
        SHAppBarMessage(ABM_SETSTATE, &mut d);
    }
}

/// Hide the taskbar after the watchdog has acknowledged that it can observe us.
/// Returns the state to restore, if we changed auto-hide ourselves.
pub fn take_over(state: u32) -> Option<u32> {
    let changed = if state & ABS_AUTOHIDE == 0 {
        set_tray_state(state | ABS_AUTOHIDE);
        Some(state)
    } else {
        None
    };
    set_visible(false);
    changed
}

pub fn restore(original_state: Option<u32>) {
    if let Some(s) = original_state { set_tray_state(s); }
    set_visible(true);
}

/// The caller must not hide the taskbar until the guard has acknowledged startup.
pub fn spawn_watchdog(original_state: Option<u32>) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("--watchdog").arg(std::process::id().to_string());
    if let Some(s) = original_state { cmd.arg(s.to_string()); }
    cmd.creation_flags(CREATE_NO_WINDOW);
    crate::taskbar_guard::spawn_ready(&mut cmd, std::time::Duration::from_secs(5))
}

/// The guard signals readiness only after it has a valid handle to the dock.
pub fn run_watchdog(pid: u32, original_state: Option<u32>) {
    use std::io::Write;
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else { return };
        if writeln!(std::io::stdout(), "{}", crate::taskbar_guard::READY).and_then(|_| std::io::stdout().flush()).is_err() {
            let _ = CloseHandle(h);
            return;
        }
        WaitForSingleObject(h, INFINITE);
        let _ = CloseHandle(h);
    }
    restore(original_state);
}

/// Preserve only the setting we actually change.
pub fn original_state(state: u32) -> Option<u32> {
    (state & ABS_AUTOHIDE == 0).then_some(state)
}

/// Reserve (or release) a strip of the monitor for the dock, on the bottom edge or — when
/// Explorer's taskbar keeps the bottom one — on the top edge. Returns the rect Windows
/// actually gave it (or `None` when nothing is reserved). Whatever comes back, the panel
/// goes inside it, so a visible taskbar can never end up on top of the panel.
///
/// `registered` remembers which edge currently holds our reservation (`Some(true)` = top),
/// because moving between edges needs a remove-and-register, not just a new position.
pub fn appbar(hwnd: HWND, monitor: RECT, height: i32, reserve: bool, at_top: bool, registered: &mut Option<bool>, last: &mut Option<crate::appbar_policy::Position>) -> Option<RECT> {
    let mut d = abd(Some(hwnd));
    d.uCallbackMessage = WM_APP_APPBAR;
    d.uEdge = if at_top { ABE_TOP } else { ABE_BOTTOM };
    d.rc = if at_top {
        RECT { left: monitor.left, top: monitor.top, right: monitor.right, bottom: monitor.top + height }
    } else {
        RECT { left: monitor.left, top: monitor.bottom - height, right: monitor.right, bottom: monitor.bottom }
    };
    unsafe {
        if reserve {
            // The shell does not move an appbar from one edge to the other: ABM_SETPOS with a
            // new uEdge leaves the old strip reserved, and that is the empty gap at the top
            // that stayed after the panel returned to the bottom. Take the old edge back first.
            if *registered == Some(!at_top) {
                d.uEdge = if at_top { ABE_BOTTOM } else { ABE_TOP };
                SHAppBarMessage(ABM_REMOVE, &mut d);
                *registered = None;
                *last = None;
                d.uEdge = if at_top { ABE_TOP } else { ABE_BOTTOM };
            }
            if registered.is_none() {
                // TaskbarCreated can also be our own broadcast, in which case Explorer
                // still has this HWND registered. ABM_NEW returns FALSE for both an
                // error and a duplicate: remove any stale registration and retry once.
                if SHAppBarMessage(ABM_NEW, &mut d) == 0 {
                    SHAppBarMessage(ABM_REMOVE, &mut d);
                    if SHAppBarMessage(ABM_NEW, &mut d) == 0 {
                        *last = None;
                        return None;
                    }
                }
                *registered = Some(at_top);
                *last = None;
            }
            SHAppBarMessage(ABM_QUERYPOS, &mut d);
            // Windows may have shrunk the strip; keep our own height on the edge we asked for.
            if at_top {
                d.rc.bottom = d.rc.top + height;
            } else {
                d.rc.top = d.rc.bottom - height;
            }
            let proposed = [d.rc.left, d.rc.top, d.rc.right, d.rc.bottom];
            // ABM_SETPOS broadcasts ABN_POSCHANGED, including to other appbars.
            // Avoid a notification feedback loop when the approved position is unchanged.
            if let Some(assigned) = crate::appbar_policy::cached_assignment(*last, proposed) {
                return Some(RECT { left: assigned[0], top: assigned[1], right: assigned[2], bottom: assigned[3] });
            }
            SHAppBarMessage(ABM_SETPOS, &mut d);
            *last = Some(crate::appbar_policy::Position {
                proposed,
                assigned: [d.rc.left, d.rc.top, d.rc.right, d.rc.bottom],
            });
            return Some(d.rc);
        } else if registered.is_some() {
            if let Some(edge_top) = *registered {
                d.uEdge = if edge_top { ABE_TOP } else { ABE_BOTTOM };
            }
            SHAppBarMessage(ABM_REMOVE, &mut d);
            *registered = None;
        }
        if !reserve { *last = None; }
    }
    None
}
