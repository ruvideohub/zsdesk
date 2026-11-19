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
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABE_BOTTOM, ABE_TOP, ABM_GETSTATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_SETSTATE,
    ABS_AUTOHIDE, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowRect, IsWindowVisible, ShowWindow, SW_HIDE, SW_SHOWNA};

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

/// Is Explorer's own taskbar on screen right now? The panel takes the edge the taskbar
/// does not use: hidden taskbar, the panel keeps the bottom; taskbar on screen, the panel
/// moves to the top of the monitor instead of stacking on top of it.
pub fn tray_visible() -> bool {
    let Some(h) = trays().first().copied() else { return false };
    unsafe {
        if !IsWindowVisible(h).as_bool() { return false; }
        // An auto-hidden taskbar keeps WS_VISIBLE and only slides off the edge: a few pixels
        // of it stay inside the monitor. A taskbar that really keeps the bottom edge shows its
        // full height, and only that one may push the panel to the top.
        let mut r = RECT::default();
        if GetWindowRect(h, &mut r).is_err() { return true; }
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() { return true; }
        let monitor = info.rcMonitor;
        (monitor.bottom.min(r.bottom) - monitor.top.max(r.top)).max(0) >= 12
    }
}

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

/// Hide the taskbar. Returns the original state if we had to switch on auto-hide.
pub fn take_over() -> Option<u32> {
    let state = tray_state();
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
    if let Some(s) = original_state {
        set_tray_state(s);
    }
    set_visible(true);
}

/// Start `laminar.exe --watchdog <pid> [state]`, a tiny process that restores the taskbar when we exit.
pub fn spawn_watchdog(original_state: Option<u32>) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if let Ok(exe) = std::env::current_exe() {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--watchdog").arg(std::process::id().to_string());
        if let Some(s) = original_state {
            cmd.arg(s.to_string());
        }
        let _ = cmd.creation_flags(CREATE_NO_WINDOW).spawn();
    }
}

/// Body of the watchdog process.
pub fn run_watchdog(pid: u32, original_state: Option<u32>) {
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(h, INFINITE);
            let _ = CloseHandle(h);
        }
    }
    restore(original_state);
}

/// Reserve (or release) a strip of the monitor for the dock, on the bottom edge or — when
/// Explorer's taskbar keeps the bottom one — on the top edge. Returns the rect Windows
/// actually gave it (or `None` when nothing is reserved). Whatever comes back, the panel
/// goes inside it, so a visible taskbar can never end up on top of the panel.
///
/// `registered` remembers which edge currently holds our reservation (`Some(true)` = top),
/// because moving between edges needs a remove-and-register, not just a new position.
pub fn appbar(hwnd: HWND, monitor: RECT, height: i32, reserve: bool, at_top: bool, registered: &mut Option<bool>) -> Option<RECT> {
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
                d.uEdge = if at_top { ABE_TOP } else { ABE_BOTTOM };
            }
            if registered.is_none() {
                SHAppBarMessage(ABM_NEW, &mut d);
                *registered = Some(at_top);
            }
            SHAppBarMessage(ABM_QUERYPOS, &mut d);
            // Windows may have shrunk the strip; keep our own height on the edge we asked for.
            if at_top {
                d.rc.bottom = d.rc.top + height;
            } else {
                d.rc.top = d.rc.bottom - height;
            }
            SHAppBarMessage(ABM_SETPOS, &mut d);
            return Some(d.rc);
        } else if registered.is_some() {
            if let Some(edge_top) = *registered {
                d.uEdge = if edge_top { ABE_TOP } else { ABE_BOTTOM };
            }
            SHAppBarMessage(ABM_REMOVE, &mut d);
            *registered = None;
        }
    }
    None
}
