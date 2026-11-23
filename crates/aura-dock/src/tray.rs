//! Tray icon hosting: the icons apps normally put beside the clock (1Password, OneDrive,
//! Discord...), shown on the dock strip and in its overflow popup and Launchpad panel.
//!
//! How apps reach the tray: `Shell_NotifyIcon` finds the top-most window whose class is
//! "Shell_TrayWnd" and sends it a WM_COPYDATA message describing the icon. Laminar creates
//! its own (never shown) window of that class and keeps it above Explorer's, so those
//! messages reach Laminar first. Laminar records each icon for the dock strip, overflow
//! popup, and Launchpad, then passes every message to Explorer unchanged, so its tray stays
//! complete when Laminar quits.
//! Messages Laminar doesn't understand (AppBar registration, icon-position queries and
//! the rest) only go to Explorer, and Explorer's answer is returned.
//!
//! Once its window is in place, Laminar broadcasts "TaskbarCreated", the message Explorer
//! sends after it restarts; apps answer it by registering their icons again, now with us.
//! Clicking an icon sends the owning app the same mouse messages Explorer would.
//! The same approach is used by the open-source Cairo and RetroBar shells (ManagedShell).

use std::cell::RefCell;

use windows::core::{w, GUID};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

macro_rules! ttrace {
    ($($arg:tt)*) => {
        if crate::dock::tracing() {
            eprintln!("[{:>7.1} ms] tray: {}", crate::dock::trace_clock(), format!($($arg)*));
        }
    };
}

// Shell_NotifyIcon messages and flags (shellapi.h).
const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETFOCUS: u32 = 3;
const NIM_SETVERSION: u32 = 4;
const NIF_MESSAGE: u32 = 0x01;
const NIF_ICON: u32 = 0x02;
const NIF_TIP: u32 = 0x04;
const NIF_STATE: u32 = 0x08;
const NIF_GUID: u32 = 0x20;
const NIF_SHOWTIP: u32 = 0x80;
const NIS_HIDDEN: u32 = 0x01;
const NIN_SELECT: u32 = WM_USER;
const NIN_POPUPOPEN: u32 = WM_USER + 6;
const NIN_POPUPCLOSE: u32 = WM_USER + 7;
/// Signature at the start of a tray WM_COPYDATA payload.
const TRAY_SIGNATURE: u32 = 0x3475_3423;
/// COPYDATASTRUCT.dwData: 0 = AppBar message, 1 = notify icon, 3 = icon position query.
const COPYDATA_NOTIFYICON: usize = 1;

/// NOTIFYICONDATAW as the shell packs it for WM_COPYDATA: handles are 32-bit so 32- and
/// 64-bit apps share one layout.
#[repr(C)]
#[derive(Clone, Copy)]
struct NotifyIconData32 {
    cb_size: u32,
    hwnd: u32,
    uid: u32,
    flags: u32,
    callback: u32,
    hicon: u32,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    version: u32, // union with uTimeout
    info_title: [u16; 64],
    info_flags: u32,
    guid: GUID,
    balloon_icon: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TrayData {
    signature: u32,
    message: u32,
    nid: NotifyIconData32,
}

/// One tray icon.
pub struct Icon {
    /// Session key used by the dock's tray-strip hit targets.
    pub key: u64,
    /// Bumped whenever the picture changes, so cached brushes know to redraw.
    pub gen: u64,
    pub owner: HWND,
    uid: u32,
    guid: Option<GUID>,
    stable_id: String,
    callback: u32,
    version: u32,
    /// Laminar's own copy of the icon (the app may destroy its handle at any time).
    pub hicon: Option<HICON>,
    pub tip: String,
    pub hidden: bool,
    show_tip: bool,
}

struct Host {
    hwnd: HWND,
    explorer: HWND,
    notify: HWND,
    notify_msg: u32,
    icons: Vec<Icon>,
    next_key: u64,
    pending: bool,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

fn handle(v: u32) -> isize {
    // 32-bit handles are sign-extended to 64 bits (the documented interop rule).
    v as i32 as isize
}

fn text(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

fn own_pid() -> u32 {
    std::process::id()
}

fn window_pid(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

fn process_image_path(h: HWND) -> Option<String> {
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    let pid = window_pid(h);
    if pid == 0 { return None; }
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(process);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]).to_lowercase())
    }
}

fn stable_id_for(owner: HWND, uid: u32, guid: Option<GUID>) -> String {
    // Stable across process restarts where possible: GUID first, otherwise executable + uid.
    if let Some(g) = guid {
        let d = g.data4;
        return format!("guid:{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            g.data1, g.data2, g.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]);
    }
    let image = process_image_path(owner).unwrap_or_else(|| format!("pid:{}", window_pid(owner)));
    let image = crate::apps::stable_path(&image).to_lowercase();
    format!("{}#{}", image, uid)
}

impl Icon {
    pub fn stable_id(&self) -> &str {
        &self.stable_id
    }

    pub fn wants_standard_tip(&self) -> bool {
        self.version < 4 || self.show_tip
    }
}

/// Explorer's real taskbar window (skipping Laminar's own stand-in).
pub fn explorer_tray() -> Option<HWND> {
    let mut prev: Option<HWND> = None;
    unsafe {
        while let Ok(h) = FindWindowExW(None, prev, w!("Shell_TrayWnd"), None) {
            if window_pid(h) != own_pid() && !is_laminar_window(h) {
                return Some(h);
            }
            prev = Some(h);
        }
    }
    None
}

/// True for a "Shell_TrayWnd" window that belongs to any Laminar process (the main dock's
/// stand-in), so helper docks and the watchdog never hide or show it by mistake.
pub fn is_laminar_window(h: HWND) -> bool {
    let pid = window_pid(h);
    if pid == own_pid() {
        return true;
    }
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(p);
        let image = String::from_utf16_lossy(&buf[..len as usize]);
        let current = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        ok && crate::tray_policy::is_dock_image(&image, &current)
    }
}

/// Start hosting. `notify` gets `notify_msg` (debounced) whenever the icons change.
pub fn start(notify: HWND, notify_msg: u32) {
    if active() { return; }
    let Some(explorer) = explorer_tray() else {
        ttrace!("no Explorer taskbar found; tray hosting off");
        return;
    };
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: w!("Shell_TrayWnd"), ..Default::default() };
        RegisterClassW(&wc);
        // Same place and size as Explorer's taskbar: some apps read the "taskbar" rectangle
        // from the first Shell_TrayWnd they find. Never shown (no WS_VISIBLE).
        let mut r = RECT::default();
        let _ = GetWindowRect(explorer, &mut r);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            w!("Shell_TrayWnd"),
            w!(""),
            WS_POPUP,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            ttrace!("couldn't create the tray window");
            return;
        };
        HOST.with(|h| {
            *h.borrow_mut() = Some(Host { hwnd, explorer, notify, notify_msg, icons: Vec::new(), next_key: 1, pending: false });
        });
        raise();
        ttrace!("hosting: our window {:?}, Explorer's {:?}", hwnd.0, explorer.0);
        broadcast_taskbar_created();
    }
}

/// Stop hosting (Explorer already has every icon, because each message was passed on).
pub fn stop() {
    let host = HOST.with(|h| h.borrow_mut().take());
    if let Some(mut host) = host {
        for i in host.icons.drain(..) {
            if let Some(ic) = i.hicon {
                unsafe {
                    let _ = DestroyIcon(ic);
                }
            }
        }
        unsafe {
            let _ = DestroyWindow(host.hwnd);
        }
    }
}

fn broadcast_taskbar_created() {
    unsafe {
        let msg = RegisterWindowMessageW(w!("TaskbarCreated"));
        let _ = SendNotifyMessageW(HWND_BROADCAST, msg, WPARAM(0), LPARAM(0));
    }
}

/// Keep our window ahead of Explorer's (Explorer can re-order its taskbar). Cheap: call
/// it every second or two.
pub fn raise() {
    HOST.with(|h| {
        let b = h.borrow();
        let Some(host) = b.as_ref() else { return };
        unsafe {
            let first = FindWindowW(w!("Shell_TrayWnd"), None).unwrap_or_default();
            if first != host.hwnd {
                let _ = SetWindowPos(host.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                let now = FindWindowW(w!("Shell_TrayWnd"), None).unwrap_or_default();
                ttrace!("raised our window above Explorer's (first now ours: {})", now == host.hwnd);
            }
        }
    });
}

/// Explorer restarted (it broadcast TaskbarCreated): follow its new taskbar window, get back
/// in front, and ask apps to register again. Our own broadcast also lands here; Explorer's
/// window is unchanged then, so nothing repeats.
pub fn on_taskbar_created() {
    let changed = HOST.with(|h| {
        let mut b = h.borrow_mut();
        let Some(host) = b.as_mut() else { return false };
        match explorer_tray() {
            Some(e) if e != host.explorer => {
                host.explorer = e;
                true
            }
            _ => false,
        }
    });
    if changed {
        ttrace!("Explorer restarted; re-registering");
        raise();
        broadcast_taskbar_created();
    }
}

/// Visible icons, in the order apps added them, with dead owners dropped.
pub fn with_icons<R>(f: impl FnOnce(&[Icon]) -> R) -> R {
    HOST.with(|h| {
        let mut b = h.borrow_mut();
        match b.as_mut() {
            Some(host) => {
                host.pending = false;
                let before = host.icons.len();
                host.icons.retain(|i| {
                    let alive = unsafe { IsWindow(Some(i.owner)).as_bool() };
                    if !alive {
                        if let Some(ic) = i.hicon {
                            unsafe {
                                let _ = DestroyIcon(ic);
                            }
                        }
                    }
                    alive
                });
                if host.icons.len() != before {
                    ttrace!("dropped {} icon(s) whose app closed without removing them", before - host.icons.len());
                }
                f(&host.icons)
            }
            None => f(&[]),
        }
    })
}

pub fn active() -> bool {
    HOST.with(|h| h.borrow().is_some())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Click {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrayInput {
    MouseMove,
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
    PopupOpen,
    PopupClose,
    Select,
    ContextMenu,
}

#[derive(Clone)]
pub struct IconSnapshot {
    pub key: u64,
    pub generation: u64,
    pub hicon: Option<HICON>,
    pub stable_id: String,
    pub tip: String,
    pub hidden: bool,
    pub standard_tip: bool,
}

pub fn snapshots() -> Vec<IconSnapshot> {
    with_icons(|icons| icons.iter().map(|i| IconSnapshot {
        key: i.key,
        generation: i.gen,
        hicon: i.hicon,
        stable_id: i.stable_id().to_string(),
        tip: i.tip.clone(),
        hidden: i.hidden,
        standard_tip: i.wants_standard_tip(),
    }).collect())
}

fn event_code(input: TrayInput) -> u32 {
    match input {
        TrayInput::MouseMove => WM_MOUSEMOVE,
        TrayInput::LeftDown => WM_LBUTTONDOWN,
        TrayInput::LeftUp => WM_LBUTTONUP,
        TrayInput::RightDown => WM_RBUTTONDOWN,
        TrayInput::RightUp => WM_RBUTTONUP,
        TrayInput::PopupOpen => NIN_POPUPOPEN,
        TrayInput::PopupClose => NIN_POPUPCLOSE,
        TrayInput::Select => NIN_SELECT,
        TrayInput::ContextMenu => WM_CONTEXTMENU,
    }
}

/// Send one callback to the owning app using the legacy or NOTIFYICON_VERSION_4 packing.
pub fn send(key: u64, input: TrayInput, at: POINT) {
    let target = HOST.with(|h| {
        let b = h.borrow();
        let icon = b.as_ref()?.icons.iter().find(|i| i.key == key)?;
        Some((icon.owner, icon.uid, icon.callback, icon.version, icon.tip.clone()))
    });
    let Some((owner, uid, callback, version, tip)) = target else { return };
    if callback == 0 {
        // One request per physical click; down/up/select arrive as a sequence.
        if !matches!(input, TrayInput::LeftDown | TrayInput::RightDown) { return; }
        ttrace!("callback for '{tip}' is not registered; asking the app to re-register");
        unsafe {
            let message = RegisterWindowMessageW(w!("TaskbarCreated"));
            let _ = PostMessageW(Some(owner), message, WPARAM(0), LPARAM(0));
        }
        return;
    }
    if version < 4 && matches!(input, TrayInput::PopupOpen | TrayInput::PopupClose) { return; }
    let message = event_code(input);
    let (wp, lp) = if version >= 4 {
        (((at.y as u16 as usize) << 16) | (at.x as u16 as usize), ((uid as usize & 0xFFFF) << 16 | message as usize) as isize)
    } else {
        (uid as usize, message as isize)
    };
    if matches!(input, TrayInput::LeftUp | TrayInput::RightUp) {
        unsafe { let _ = AllowSetForegroundWindow(window_pid(owner)); }
    }
    unsafe {
        if let Err(e) = PostMessageW(Some(owner), callback, WPARAM(wp), LPARAM(lp)) {
            ttrace!("'{tip}': message {message:#x} refused ({e:?}) — a closed window, or the app runs elevated");
        }
    }
}

/// Click an icon the way Explorer's tray does: mouse down, up, then its versioned selection event.
pub fn click(key: u64, kind: Click, at: POINT) {
    send(key, TrayInput::MouseMove, at);
    match kind {
        Click::Left => {
            send(key, TrayInput::LeftDown, at);
            send(key, TrayInput::LeftUp, at);
            let version = with_icons(|icons| icons.iter().find(|i| i.key == key).map(|i| i.version).unwrap_or(0));
            if version >= 3 { send(key, TrayInput::Select, at); }
        }
        Click::Right => {
            send(key, TrayInput::RightDown, at);
            send(key, TrayInput::RightUp, at);
            let version = with_icons(|icons| icons.iter().find(|i| i.key == key).map(|i| i.version).unwrap_or(0));
            if version >= 3 { send(key, TrayInput::ContextMenu, at); }
        }
    }
}

fn changed(host: &mut Host) {
    if !host.pending {
        host.pending = true;
        unsafe {
            let _ = PostMessageW(Some(host.notify), host.notify_msg, WPARAM(0), LPARAM(0));
        }
    }
}

/// Apply one Shell_NotifyIcon message. Returns what Explorer would: TRUE on success.
fn apply(host: &mut Host, msg: u32, n: &NotifyIconData32) -> bool {
    let owner = HWND(handle(n.hwnd) as _);
    let guid = if n.flags & NIF_GUID != 0 { Some(n.guid) } else { None };
    let pos = host.icons.iter().position(|i| match guid {
        Some(g) => i.guid == Some(g),
        None => i.guid.is_none() && i.owner == owner && i.uid == n.uid,
    });
    match msg {
        NIM_ADD | NIM_MODIFY => {
            // An icon we already have (Explorer answers FALSE to a repeat add) still updates
            // its details: icons first seen through a modify may lack the click message.
            let repeat_add = pos.is_some() && msg == NIM_ADD;
            let idx = match (pos, msg) {
                (Some(p), _) => p,
                (None, NIM_MODIFY) if n.flags & NIF_ICON == 0 => return false,
                (None, _) => {
                    // A modify for an icon we never saw counts as an add (the app registered
                    // before Laminar started and didn't answer TaskbarCreated).
                    host.icons.push(Icon {
                        key: host.next_key,
                        gen: 0,
                        owner,
                        uid: n.uid,
                        guid,
                        stable_id: stable_id_for(owner, n.uid, guid),
                        callback: 0,
                        version: 0,
                        hicon: None,
                        tip: String::new(),
                        hidden: false,
                        show_tip: false,
                    });
                    host.next_key += 1;
                    host.icons.len() - 1
                }
            };
            let i = &mut host.icons[idx];
            i.uid = crate::tray_policy::registration_uid(msg == NIM_ADD, guid.is_some(), i.uid, n.uid);
            if !owner.is_invalid() && i.owner != owner {
                i.owner = owner;
                i.stable_id = stable_id_for(i.owner, i.uid, i.guid);
            }
            if n.flags & NIF_MESSAGE != 0 {
                i.callback = n.callback;
            }
            if n.flags & NIF_ICON != 0 {
                let src = HICON(handle(n.hicon) as _);
                let copy = if src.is_invalid() { None } else { unsafe { CopyIcon(src).ok() } };
                if let Some(old) = std::mem::replace(&mut i.hicon, copy) {
                    unsafe {
                        let _ = DestroyIcon(old);
                    }
                }
                i.gen += 1;
            }
            if n.flags & NIF_TIP != 0 {
                i.tip = text(&n.tip);
            }
            if n.flags & (NIF_TIP | NIF_SHOWTIP) != 0 {
                i.show_tip = n.flags & NIF_SHOWTIP != 0;
            }
            if n.flags & NIF_STATE != 0 && n.state_mask & NIS_HIDDEN != 0 {
                i.hidden = n.state & NIS_HIDDEN != 0;
            }
            if pos.is_none() {
                ttrace!("icon added: '{}' (hidden={}, has picture={}, click message={})", i.tip.lines().next().unwrap_or(""), i.hidden, i.hicon.is_some(), i.callback != 0);
            }
            changed(host);
            !repeat_add
        }
        NIM_DELETE => match pos {
            Some(p) => {
                let i = host.icons.remove(p);
                if let Some(ic) = i.hicon {
                    unsafe {
                        let _ = DestroyIcon(ic);
                    }
                }
                changed(host);
                true
            }
            None => false,
        },
        NIM_SETVERSION => match pos {
            Some(p) => {
                host.icons[p].version = n.version;
                changed(host);
                true
            }
            None => false,
        },
        NIM_SETFOCUS => true,
        _ => false,
    }
}

/// Pass a message on to Explorer's taskbar and return its answer (never blocking the dock
/// for long if Explorer is busy).
fn forward(explorer: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let mut result = 0usize;
    unsafe {
        let ok = SendMessageTimeoutW(explorer, msg, wp, lp, SMTO_ABORTIFHUNG | SMTO_NORMAL, 2000, Some(&mut result));
        if ok.0 == 0 {
            return LRESULT(0);
        }
    }
    LRESULT(result as isize)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let explorer = HOST.with(|h| h.borrow().as_ref().map(|x| x.explorer));
    let Some(explorer) = explorer else { return DefWindowProcW(hwnd, msg, wp, lp) };
    match msg {
        WM_COPYDATA => {
            let cds = lp.0 as *const COPYDATASTRUCT;
            if !cds.is_null() && (*cds).dwData == COPYDATA_NOTIFYICON && !(*cds).lpData.is_null() {
                let cds = &*cds;
                let mut data: TrayData = std::mem::zeroed();
                let n = (cds.cbData as usize).min(std::mem::size_of::<TrayData>());
                std::ptr::copy_nonoverlapping(cds.lpData as *const u8, &mut data as *mut TrayData as *mut u8, n);
                if data.signature == TRAY_SIGNATURE {
                    let ours = HOST.with(|h| h.borrow_mut().as_mut().map(|host| apply(host, data.message, &data.nid)).unwrap_or(false));
                    // Explorer keeps its own copy (its answer for an icon it already had
                    // would be FALSE; ours is the one the app should see).
                    let _ = forward(explorer, msg, wp, lp);
                    return LRESULT(ours as isize);
                }
            }
            forward(explorer, msg, wp, lp)
        }
        // Never let anything show this stand-in window.
        WM_WINDOWPOSCHANGING => {
            let p = lp.0 as *mut WINDOWPOS;
            if !p.is_null() {
                (*p).flags &= !SWP_SHOWWINDOW;
            }
            LRESULT(0)
        }
        WM_NCCREATE | WM_CREATE | WM_NCCALCSIZE | WM_DESTROY | WM_NCDESTROY | WM_GETMINMAXINFO | WM_MOVE | WM_SIZE
        | WM_WINDOWPOSCHANGED | WM_SHOWWINDOW | WM_ACTIVATEAPP | WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        // Commands and private messages apps send to "the taskbar" go to the real one.
        // (Not registered messages, 0xC000 and up: those are broadcasts such as our own
        // TaskbarCreated, which Explorer's taskbar must not receive from us.)
        m if m == WM_COMMAND || (WM_USER..0xC000).contains(&m) => forward(explorer, msg, wp, lp),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
