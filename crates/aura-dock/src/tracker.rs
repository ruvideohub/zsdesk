//! Running-window tracking: which windows would appear on the Windows taskbar,
//! which app each belongs to, and which is in front.
//!
//! Event-driven: WinEvent hooks tell us when windows show, hide, cloak,
//! minimize or take focus; the dock then rescans (debounced). No polling.

use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::dock::WM_APP_RESCAN;

#[derive(Clone, Debug)]
pub struct Win {
    pub hwnd: HWND,
    /// Lower-case app identity: AppUserModelID if the app has one, else the exe path.
    pub app: String,
    /// Lower-case full exe path of the owning process.
    pub exe: String,
    // Kept for the window list in the upcoming styled menus and previews.
    #[allow(dead_code)]
    pub minimized: bool,
    #[allow(dead_code)]
    pub title: String,
}

/// Remembers each window's identity so a rescan doesn't re-query every process.
#[derive(Default)]
pub struct Tracker {
    identity: HashMap<isize, (u32, String, String)>,
}

static NOTIFY: AtomicIsize = AtomicIsize::new(0);

/// Hook the window events that can change the dock (out-of-context: callbacks
/// arrive on this thread through its message loop).
pub fn install_hooks(notify: HWND) -> Vec<HWINEVENTHOOK> {
    NOTIFY.store(notify.0 as isize, Ordering::Relaxed);
    let ranges = [
        (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
        (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
        (EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZEEND),
        (EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE),
        (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        (EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE),
    ];
    ranges
        .iter()
        .map(|(a, b)| unsafe {
            SetWinEventHook(*a, *b, None, Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)
        })
        .collect()
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 || hwnd.is_invalid() {
        return;
    }
    // Only top-level windows matter (child controls show and hide constantly).
    if event != EVENT_SYSTEM_FOREGROUND && GetAncestor(hwnd, GA_ROOT) != hwnd {
        return;
    }
    let target = HWND(NOTIFY.load(Ordering::Relaxed) as _);
    let _ = PostMessageW(Some(target), WM_APP_RESCAN, WPARAM(event as usize), LPARAM(hwnd.0 as isize));
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn is_explorer_taskbar(h: HWND) -> bool {
    matches!(class_name(h).as_str(), "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") && !crate::tray::is_laminar_window(h)
}

fn cloaked(h: HWND) -> bool {
    let mut v: u32 = 0;
    unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut u32 as _, 4).is_ok() && v != 0 }
}

fn title(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// The same rules the Windows taskbar uses, approximately.
fn is_taskbar_window(h: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(h).as_bool() || cloaked(h) {
            return false;
        }
        let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(h, GWL_EXSTYLE) as u32);
        if ex.contains(WS_EX_APPWINDOW) {
            return true;
        }
        if ex.contains(WS_EX_TOOLWINDOW) || ex.contains(WS_EX_NOACTIVATE) {
            return false;
        }
        if GetWindow(h, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false) {
            return false;
        }
        if title(h).is_empty() {
            return false;
        }
        !matches!(
            class_name(h).as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "AuraExplorerDock" | "Windows.UI.Core.CoreWindow"
        )
    }
}

fn window_aumid(h: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(h).ok()?;
        let mut v = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let s = PropVariantToStringAlloc(&v).ok().map(|p| {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            s
        });
        let _ = PropVariantClear(&mut v);
        s.filter(|s| !s.is_empty())
    }
}

/// (packaged AppUserModelID, exe path) of the window's process.
fn process_info(h: HWND) -> (Option<String>, String) {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (None, String::new());
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let aumid = if GetApplicationUserModelId(proc, &mut len, Some(PWSTR(buf.as_mut_ptr()))).is_ok() {
            Some(String::from_utf16_lossy(&buf[..len.saturating_sub(1) as usize]))
        } else {
            None
        };
        let mut len = buf.len() as u32;
        let exe = if QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            String::new()
        };
        let _ = CloseHandle(proc);
        (aumid, exe)
    }
}

unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> BOOL {
    let v = &mut *(lp.0 as *mut Vec<HWND>);
    v.push(h);
    true.into()
}

impl Tracker {
    /// All taskbar-style windows, front to back (most recently used first).
    pub fn scan(&mut self) -> Vec<Win> {
        let mut all: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut _ as isize));
        }
        let mut out = Vec::new();
        let mut seen = HashMap::new();
        for h in all {
            if !is_taskbar_window(h) {
                continue;
            }
            let key = h.0 as isize;
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(h, Some(&mut pid)); }
            let cached = self.identity.get(&key)
                .filter(|(owner, app, _)| crate::tray_policy::reuse_identity(*owner, pid, app))
                .map(|(_, app, exe)| (app.clone(), exe.clone()));
            let (app, exe) = match cached {
                Some(v) => v,
                None => {
                    let (paumid, exe) = process_info(h);
                    let exe = crate::apps::stable_path(&exe);
                    let app = window_aumid(h).or(paumid).unwrap_or_else(|| exe.clone());
                    let v = (app.to_lowercase(), exe.to_lowercase());
                    // A transient failure must be retried on the next scan, not cached forever.
                    if pid != 0 && !v.0.is_empty() { self.identity.insert(key, (pid, v.0.clone(), v.1.clone())); }
                    else { self.identity.remove(&key); }
                    v
                }
            };
            seen.insert(key, ());
            if app.is_empty() {
                continue;
            }
            out.push(Win { hwnd: h, app, exe, minimized: unsafe { IsIconic(h).as_bool() }, title: title(h) });
        }
        self.identity.retain(|k, _| seen.contains_key(k));
        out
    }
}

/// Map a focused window or dialog to its closest taskbar-eligible owner.
/// An owned WS_EX_APPWINDOW has its own button and must not be collapsed into
/// the root owner. For untracked windows preserve the old root-owner fallback.
pub fn taskbar_ancestor(h: HWND) -> HWND {
    unsafe {
        let root = GetAncestor(h, GA_ROOT);
        if root.is_invalid() { return HWND::default(); }
        let mut current = root;
        // Windows normally prevents ownership cycles; bound traversal defensively
        // against destroyed/reused HWNDs while another process changes its windows.
        for _ in 0..128 {
            if is_taskbar_window(current) { return current; }
            let Ok(owner) = GetWindow(current, GW_OWNER) else { break };
            if owner.is_invalid() || owner == current { break; }
            current = owner;
        }
        GetAncestor(root, GA_ROOTOWNER)
    }
}

/// The taskbar-eligible window represented by the actual foreground window.
pub fn foreground() -> HWND {
    taskbar_ancestor(unsafe { GetForegroundWindow() })
}

/// Use a visible owned popup only if it belongs to this taskbar button; a
/// separately tracked WS_EX_APPWINDOW must not hijack its owner's activation.
fn popup_target(h: HWND, popup: HWND) -> HWND {
    if popup != h && !popup.is_invalid()
        && unsafe { IsWindowVisible(popup).as_bool() }
        && taskbar_ancestor(popup) == h
    { popup } else { h }
}

/// GetLastActivePopup returns the input unchanged for owned windows. Query the
/// root owner instead and accept the result only when it belongs to this button.
/// If a modal dialog disabled the selected window, also try its enabled popup.
fn activation_target(h: HWND) -> HWND {
    use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
    unsafe {
        let root = GetAncestor(h, GA_ROOTOWNER);
        let last = if root.is_invalid() { h } else { GetLastActivePopup(root) };
        let target = popup_target(h, last);
        if target != h { return target; }
        if !IsWindowEnabled(h).as_bool() {
            if let Ok(popup) = GetWindow(h, GW_ENABLEDPOPUP) {
                return popup_target(h, popup);
            }
        }
        h
    }
}

/// Bring a window or its active dialog to the front, restoring it if minimized.
pub fn activate(h: HWND) {
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindowAsync(h, SW_RESTORE);
        }
        let popup = activation_target(h);
        let _ = SetForegroundWindow(popup);
    }
}

/// Bring a window forward while another app is in charge of input (during a drag, the
/// app being dragged from is). Windows only lets the process that sent the last input
/// change the foreground window, so first send an empty mouse input (no movement, no
/// buttons; the drag carries on). PowerToys uses the same step.
pub fn activate_during_drag(h: HWND) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_MOUSE};
    unsafe {
        let input = INPUT { r#type: INPUT_MOUSE, ..Default::default() };
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        if IsIconic(h).as_bool() {
            let _ = ShowWindowAsync(h, SW_RESTORE);
        }
        let popup = activation_target(h);
        SetForegroundWindow(popup).as_bool()
    }
}

pub fn minimize(h: HWND) {
    unsafe {
        let _ = ShowWindowAsync(h, SW_MINIMIZE);
    }
}

/// Politely ask a window to close (the app can still prompt to save).
pub fn close(h: HWND) {
    unsafe {
        let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;

    /// Small real USER32 windows, shown without activation and destroyed even on panic.
    struct Windows(Vec<HWND>);
    impl Windows {
        fn new() -> Self { Self(Vec::new()) }
        fn window(&mut self, owner: Option<HWND>, ex: WINDOW_EX_STYLE, visible: bool) -> HWND {
            let h = unsafe {
                CreateWindowExW(ex, w!("STATIC"), w!("Dock tracker test"), WS_POPUP,
                    -30000, -30000, 1, 1, owner, None, None, None)
            }.expect("create test window");
            self.0.push(h);
            if visible { unsafe { let _ = ShowWindow(h, SW_SHOWNOACTIVATE); } }
            h
        }
    }
    impl Drop for Windows {
        fn drop(&mut self) {
            for h in self.0.drain(..).rev() { unsafe { let _ = DestroyWindow(h); } }
        }
    }

    #[test]
    fn focus_maps_to_closest_taskbar_window_not_always_root_owner() {
        let mut windows = Windows::new();
        let root = windows.window(None, WINDOW_EX_STYLE(0), true);
        let dialog = windows.window(Some(root), WINDOW_EX_STYLE(0), true);
        let separate = windows.window(Some(root), WS_EX_APPWINDOW, true);
        let nested = windows.window(Some(separate), WINDOW_EX_STYLE(0), true);
        assert!(is_taskbar_window(root));
        assert!(!is_taskbar_window(dialog));
        assert!(is_taskbar_window(separate));
        assert!(!is_taskbar_window(nested));
        assert_eq!(taskbar_ancestor(root), root);
        assert_eq!(taskbar_ancestor(dialog), root);
        assert_eq!(taskbar_ancestor(separate), separate);
        assert_eq!(taskbar_ancestor(nested), separate);
        assert!(taskbar_ancestor(HWND::default()).is_invalid());
    }

    #[test]
    fn disabled_owned_appwindow_uses_its_enabled_dialog() {
        use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
        let mut windows = Windows::new();
        let root = windows.window(None, WINDOW_EX_STYLE(0), true);
        let separate = windows.window(Some(root), WS_EX_APPWINDOW, true);
        let dialog = windows.window(Some(separate), WINDOW_EX_STYLE(0), true);
        unsafe { let _ = EnableWindow(separate, false); }
        assert_eq!(activation_target(separate), dialog);
        assert_eq!(popup_target(root, dialog), root);
    }

    #[test]
    fn activation_prefers_own_visible_dialog_but_not_another_taskbar_button() {
        let mut windows = Windows::new();
        let root = windows.window(None, WINDOW_EX_STYLE(0), true);
        let dialog = windows.window(Some(root), WINDOW_EX_STYLE(0), true);
        let separate = windows.window(Some(root), WS_EX_APPWINDOW, true);
        let nested = windows.window(Some(separate), WINDOW_EX_STYLE(0), true);
        let hidden = windows.window(Some(root), WINDOW_EX_STYLE(0), false);
        assert_eq!(popup_target(root, dialog), dialog);
        assert_eq!(popup_target(root, separate), root);
        assert_eq!(popup_target(root, nested), root);
        assert_eq!(popup_target(separate, nested), nested);
        assert_eq!(popup_target(root, hidden), root);
        assert_eq!(popup_target(root, HWND::default()), root);
    }
}
