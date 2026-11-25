//! Keep desktop cards below *all* visible normal windows, not just the foreground one.
//! Explorer's desktop hosts are excluded so the cards remain above the desktop. Windows
//! controls Show Desktop and shell ordering; this remains best effort for shell transitions.
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowLongPtrW,
    IsWindowVisible, SetWindowPos, GWL_EXSTYLE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    WINDOW_EX_STYLE, WS_EX_TOPMOST};

unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> BOOL {
    let all = &mut *(lp.0 as *mut Vec<HWND>);
    all.push(h);
    true.into()
}

fn eligible(h: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(h).as_bool() { return false; }
        let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(h, GWL_EXSTYLE) as u32);
        if ex.contains(WS_EX_TOPMOST) { return false; }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, 4).is_ok() && cloaked != 0 { return false; }
        let mut name = [0u16; 80];
        let n = GetClassNameW(h, &mut name).max(0) as usize;
        let class = String::from_utf16_lossy(&name[..n.min(name.len())]);
        !matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" |
            "AuraExplorerDesktopClock" | "AuraExplorerDesktopSystem" | "SysShadow")
    }
}

/// Reorder only if a normal window has slipped below the card. Does not activate it.
pub fn keep_below_windows(card: HWND) {
    let mut windows = Vec::new();
    unsafe { let _ = EnumWindows(Some(collect), LPARAM(&mut windows as *mut _ as isize)); }
    let Some(index) = windows.iter().position(|&h| h == card) else { return };
    let eligible: Vec<bool> = windows.iter().enumerate().map(|(i, &h)| i > index && eligible(h)).collect();
    if let Some(last) = crate::desktop_layer_policy::last_below(index, &eligible) {
        unsafe { let _ = SetWindowPos(card, Some(windows[last]), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE); }
    }
}
