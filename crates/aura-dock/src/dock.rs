//! The dock window and its Composition visual tree.
//!
//! Magnification is done with *expression animations*: Rust only writes the
//! pointer position into a property set, and the compositor computes every
//! icon's scale and position on the GPU each frame. The UI thread sleeps
//! unless something happens (pointer, click, a window event, a settings save).

use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::time::Instant;

use windows::core::{w, Interface, Result, HSTRING};
use windows::Foundation::TimeSpan;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmFlush, DwmSetWindowAttribute, DWMWA_EXCLUDED_FROM_PEEK, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, GetMonitorInfoW, MonitorFromPoint, SetWindowRgn, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::Storage::FileSystem::{FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_LAST_WRITE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::{Vector2, Vector3};

use crate::apps;
use crate::config::{self, Config, Pin, SEPARATOR};
use crate::gfx::Gfx;
use crate::iconcache::IconCache;
use crate::menu::{Menu, MenuItem};
use crate::preview::{Hit as PreviewHit, Preview};
use crate::stack::{StackApp, StackView};
use crate::launchpad::{self as lp, Act, Launchpad};
use crate::services::{self, ControlState, Reply, Req, Services, WM_APP_SVC};
use laminar_sys::apps::AppEntry;
use laminar_sys::files::FileHit;
use laminar_sys::stats::Stats;
use laminar_sys::weather::Weather;
use crate::taskbar;
use crate::tracker::{self, Tracker, Win};
use crate::{metrics, Options};

fn report_settings_error(hwnd: HWND, error: &std::io::Error) {
    eprintln!("dock settings were not saved: {error}");
    let message: Vec<u16> = format!("Не удалось сохранить настройки панели. Существующий файл не изменён.\n\n{}", error)
        .encode_utf16().chain(Some(0)).collect();
    unsafe { let _ = MessageBoxW(Some(hwnd), windows::core::PCWSTR(message.as_ptr()), w!("AuraDock"), MB_OK | MB_ICONERROR); }
}

/// Defined in Win32::UI::Controls; declared here to avoid pulling in that whole module.
const WM_MOUSELEAVE: u32 = 0x02A3;
pub const WM_APP_RESCAN: u32 = WM_APP + 1;
pub const WM_APP_APPBAR: u32 = WM_APP + 2;
const WM_APP_CONFIG: u32 = WM_APP + 3;
/// Another process asked for the settings page (`AuraDock.exe --settings`).
const WM_APP_SETTINGS: u32 = WM_APP + 8;

const T_SHRINK: usize = 1;
const T_MEASURE: usize = 2;
const T_EXIT: usize = 3;
const T_SWEEP: usize = 4;
const T_RESCAN: usize = 5;
const T_REVEAL: usize = 6;
const T_HIDE: usize = 7;
const T_CONFIG: usize = 8;
const T_TUCKED: usize = 9;
const T_GHOST: usize = 10;
const T_PREVIEW: usize = 11;
const T_PREVIEW_HIDE: usize = 12;
const T_MERGE: usize = 13;
const T_FILES: usize = 14;
const T_LP_TICK: usize = 15;
const T_WEATHER: usize = 16;
/// Keeps the tray stand-in window ahead of Explorer's.
const T_TRAY_RAISE: usize = 17;
/// Rebuilds tray-strip, overflow and Launchpad views after a burst of icon updates.
const T_TRAY_RENDER: usize = 18;
/// A drag rested on a running app: show its window tiles.
const T_DRAG_PREVIEW: usize = 19;
/// A drag rested on a window tile: bring that window forward.
const T_DRAG_SWITCH: usize = 20;
/// The desktop clock card: one second, redraws only when its text changes.
pub const WM_APP_WINKEY: u32 = WM_APP + 4;
/// The desktop clock card was right-clicked: open its menu.
pub const WM_APP_CLOCK_MENU: u32 = WM_APP + 12;
/// The desktop clock card was dragged to a corner: wparam is the corner code.
pub const WM_APP_CLOCK_MOVED: u32 = WM_APP + 13;
/// The desktop load card's own menu, asked for with a right-click on the card.
pub const WM_APP_SYSTEM_MENU: u32 = WM_APP + 14;
/// Where the load card was let go, in physical px (wparam = x, lparam = y).
pub const WM_APP_SYSTEM_MOVED: u32 = WM_APP + 15;
/// Re-check whether the pointer is over the dock (after a drag that bypassed hover tracking).
const WM_APP_RESYNC_HOVER: u32 = WM_APP + 5;
/// The tray icons changed (see tray.rs).
const WM_APP_TRAY: u32 = WM_APP + 6;
/// An update check finished (see updater.rs).
const WM_APP_UPDATE: u32 = WM_APP + 7;
/// Test control (only with LAMINAR_TESTCTL set): lParam is a screen point the dock should
/// treat as the pointer position. A test driver posts this, then ordinary mouse messages,
/// so the dock can be exercised (and captured with PrintWindow) without a real mouse,
/// for example while the session is locked.
const WM_APP_TESTCTL: u32 = WM_APP + 40;

/// A window that belongs to this process (Launchpad, menus, previews, the dock itself).
fn is_own_window(h: HWND) -> bool {
    if h.0.is_null() {
        return true;
    }
    let mut pid = 0u32;
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid == std::process::id()
}

/// LAMINAR_TESTCTL: pointer driven by WM_APP_TESTCTL instead of the real cursor.
fn testctl() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("LAMINAR_TESTCTL").is_ok())
}

static FAKE_CURSOR: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

/// LAMINAR_TESTCTL: a plain stand-in for the desktop behind the glass. In a capture of a
/// locked session the host backdrop shows this window's own stale frames instead.
fn test_backdrop(c: &Compositor) -> Option<CompositionBrush> {
    if !testctl() {
        return None;
    }
    c.CreateColorBrushWithColor(windows::UI::Color { A: 255, R: 38, G: 52, B: 78 }).ok()?.cast().ok()
}

/// The pointer position: the real cursor, or the test driver's point under LAMINAR_TESTCTL.
fn get_cursor_pos(p: &mut POINT) -> windows::core::Result<()> {
    if testctl() {
        if let Some((x, y)) = FAKE_CURSOR.lock().ok().and_then(|g| *g) {
            *p = POINT { x, y };
            return Ok(());
        }
    }
    unsafe { GetCursorPos(p) }
}
/// Check for a new Laminar (first soon after start, then every few hours).
const T_UPDATE_CHECK: usize = 21;

// Menu command ids
const CMD_NEW: usize = 10;
const CMD_PIN: usize = 11;
const CMD_CLOSE: usize = 12;
const CMD_AUTOHIDE: usize = 20;
const CMD_TASKBAR: usize = 21;
const CMD_SETTINGS: usize = 22;
const CMD_QUIT: usize = 23;
const CMD_MAGNIFY: usize = 24;
const CMD_ADD_SEP: usize = 25;
const CMD_TRASH_OPEN: usize = 26;
const CMD_TASKMGR: usize = 27;
const CMD_WINSETTINGS: usize = 28;
const CMD_EXPLORER: usize = 29;
const CMD_TERMINAL: usize = 30;
const CMD_WINDOW_BASE: usize = 100;
const CMD_REMOVE_SEP_BASE: usize = 1000;
const CMD_STACK_OPEN: usize = 31;
const CMD_STACK_UNGROUP: usize = 32;
const CMD_STACK_REMOVE: usize = 33;
const CMD_NEW_STACK: usize = 34;
const CMD_STACK_RENAME: usize = 35;
const CMD_ADD_TO_STACK_BASE: usize = 2000;
const CMD_POWER_LOCK: usize = 40;
const CMD_POWER_SIGNOUT: usize = 41;
const CMD_POWER_SLEEP: usize = 42;
const CMD_POWER_RESTART: usize = 43;
const CMD_POWER_SHUTDOWN: usize = 44;
const CMD_LP_OPEN: usize = 50;
/// Open the control centre (the drop-down above the panel).
const CMD_CONTROL_CENTER: usize = 49;
const CMD_LP_ADMIN: usize = 51;
const CMD_LP_DOCK: usize = 52;
const CMD_LP_PIN: usize = 53;
const CMD_LP_REVEAL: usize = 54;
const CMD_UPDATE_NOW: usize = 60;
const CMD_UPDATE_NOTES: usize = 61;
const CMD_UPDATE_TOMORROW: usize = 62;
const CMD_UPDATE_WEEK: usize = 63;
const CMD_UPDATE_SKIP: usize = 64;
const CMD_UPDATE_CHECK: usize = 65;
const CMD_WIDGET_REMOVE: usize = 70;
const CMD_WIDGET_TO_LP: usize = 71;
const CMD_WIDGET_CONNECT: usize = 72;
const CMD_WIDGET_RESET: usize = 73;
const CMD_LPW_REMOVE: usize = 74;
const CMD_LPW_TO_DOCK: usize = 75;
const CMD_LPW_CONNECT: usize = 76;
const CMD_LPW_UP: usize = 77;
const CMD_LPW_DOWN: usize = 78;
const CMD_LPW_RESET: usize = 79;
/// The desktop clock card's own menu: corners (base + ClockCorner::code), sizes.
const CMD_CLOCK_CORNER_BASE: usize = 3600;
const CMD_CLOCK_SCALE_BASE: usize = 3610;
const CMD_CLOCK_24H: usize = 3620;
const CMD_CLOCK_DATE: usize = 3621;
const CMD_CLOCK_SECONDS: usize = 3622;
const CMD_CLOCK_HIDE: usize = 3623;
/// The panel's own switch for the desktop clock (the card's menu can only hide it).
const CMD_CLOCK_DESKTOP: usize = 3624;
/// The card keeps its place while dragging is off.
const CMD_CLOCK_LOCK: usize = 3625;
/// The desktop load card: switch, corner, size, lock, hide.
const CMD_SYSTEM_DESKTOP: usize = 3630;
const CMD_SYSTEM_CORNER_BASE: usize = 3640;
const CMD_SYSTEM_SCALE_BASE: usize = 3650;
const CMD_SYSTEM_LOCK: usize = 3660;
const CMD_SYSTEM_HIDE: usize = 3661;
/// Widgets on the panel: the section header's list once, then the gallery.
const CMD_WIDGET_ADD: usize = 81;
const CMD_WIDGET_TOGGLE_BASE: usize = 3700;
/// Widgets one click away from the panel's own menu: the everyday ones.
const DOCK_WIDGETS: [&str; 9] = [
    "clock",
    "weather-hourly",
    "month",
    "resource-monitor",
    "battery",
    "now-playing-compact",
    "network",
    "control-tiles",
    "control-center",
];
/// The card's three sizes, as multipliers.
const CLOCK_SCALES: [f32; 3] = [0.85, 1.0, 1.2];
const CLOCK_SIZE_NAMES: [&str; 3] = ["Compact", "Normal", "Large"];
/// Timer length menu: base + minutes.
const CMD_TIMER_MIN_BASE: usize = 3000;
const TIMER_MINUTES: [usize; 7] = [1, 5, 10, 15, 25, 45, 60];
/// Widget refresh (once a second).
const T_WIDGETS: usize = 22;
/// Holding a widget card lifts it (ready to drag) after this long.
const T_LP_HOLD: usize = 23;
/// Tray hover tooltip / app-owned popup delay.
const T_TRAY_TIP: usize = 24;
/// Debounce Shell appbar position notifications.
const T_APPBAR: usize = 25;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Launch,
    App,
    Separator,
    Stack,
    /// A new Laminar version is downloaded and ready to install.
    Update,
    /// A widget pinned to the dock (key "::widget{pin index}:{widget id}").
    Widget,
    /// A notification-area icon, addressed by its session key.
    Tray(u64),
    /// Opens the overflow popup for icons not shown on the strip.
    TrayOverflow,
}

/// The widget a dock item shows (key "::widget{pin}:{id}:{s|m|l}").
fn widget_of(key: &str) -> Option<&'static crate::widgets::Def> {
    let rest = key.strip_prefix("::widget")?;
    crate::widgets::catalog::get(rest.split(':').nth(1)?)
}

/// The size a dock widget item is shown at.
fn widget_size(key: &str) -> crate::widgets::Size {
    use crate::widgets::Size;
    match key.rsplit(':').next() {
        Some("s") => Size::Small,
        Some("l") => Size::Large,
        Some("m") => Size::Medium,
        _ => widget_of(key).map(|d| d.size).unwrap_or_default(),
    }
}

/// Widget id of the control centre button: it never leaves the right-hand end.
const CONTROL_CENTER: &str = "control-center";

/// How much wide widgets magnify, relative to icons: enough to read as "lifted" without
/// shoving the rest of the dock aside. Icon-sized (small) widgets magnify like icons.
const WIDGET_MAG: f32 = 0.75;

/// Pin index of a widget item.
fn widget_pin(key: &str) -> Option<usize> {
    key.strip_prefix("::widget")?.split_once(':')?.0.parse().ok()
}

#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    pub kind: Kind,
    /// App id (as pinned or as running), or "::launch" / "::trash" / "::sep…".
    pub key: String,
    pub pinned: bool,
}

impl Item {
    fn is_icon(&self) -> bool {
        !matches!(self.kind, Kind::Separator | Kind::Widget)
    }
}

/// Layout in physical pixels for the current DPI and settings.
struct Geo {
    b: f32,
    m: f32,
    g: f32,
    p: f32,
    r: f32,
    margin: f32,
    range: f32,
    sep_w: f32,
    win_w: f32,
    win_h: f32,
    x0: f32,
    y0: f32,
    shelf_w: f32,
    shelf_h: f32,
    left: Vec<f32>,
    width: Vec<f32>,
    icon: Vec<bool>,
    /// Widget items.
    wid: Vec<bool>,
    /// The panel hangs from the top edge of the monitor: the shelf sits at the top of the
    /// window and items grow downwards. False on the bottom edge, where it grew upwards.
    top: bool,
    /// Per item: drawn width before magnification (icon size, a widget's width, 0 for
    /// separators), magnification strength (0: never), and how far from its centre the
    /// pointer can be and still count as "on" it (half the extra width of a widget).
    vw: Vec<f32>,
    kk: Vec<f32>,
    flat: Vec<f32>,
    hover_lr: (i32, i32),
}

impl Geo {
    fn new(items: &[Item], cfg: &Config, scale: f32, win_w: f32, top: bool) -> Self {
        let b = cfg.icon_size * scale;
        let m = if cfg.magnify { cfg.magnified_size * scale } else { b };
        let (g, p) = (cfg.icon_spacing * scale, cfg.padding * scale);
        let sep_w = 13.0 * scale + g;
        let shelf_h = b + 2.0 * p;
        let r = (cfg.corner_radius * scale).min(shelf_h / 2.0);
        let margin = 8.0 * scale;
        let label_room = 40.0 * scale;
        let win_h = ((m - b) + shelf_h + margin + label_room).ceil();
        let k = m / b - 1.0;
        let (mut left, mut width, mut x) = (Vec::new(), Vec::new(), 0.0);
        let (mut vw, mut kk, mut flat) = (Vec::new(), Vec::new(), Vec::new());
        for it in items {
            let (w, v, kx) = if it.kind == Kind::Widget {
                let ww = widget_of(&it.key).map(|d| crate::widgets::dock_width(d, widget_size(&it.key), b)).unwrap_or(b);
                (ww + g, ww, if ww <= b * 1.05 { k } else { k * WIDGET_MAG })
            } else if it.is_icon() {
                (b + g, b, k)
            } else {
                (sep_w, 0.0, 0.0)
            };
            left.push(x);
            width.push(w);
            vw.push(v);
            kk.push(kx);
            flat.push(((v - b) / 2.0).max(0.0));
            x += w;
        }
        let shelf_w = x + 2.0 * p;
        let x0 = ((win_w - shelf_w) / 2.0).round();
        // The label room and the magnification headroom sit on the side the panel grows
        // away from the screen edge: above the shelf at the bottom, below it at the top.
        let y0 = if top { margin } else { win_h - margin - shelf_h };
        let mut geo = Geo {
            b, m, g, p, r, margin,
            range: cfg.magnify_range * (b + g),
            sep_w, win_w, win_h, x0, y0, shelf_w, shelf_h, left, width, top,
            icon: items.iter().map(|i| i.is_icon()).collect(),
            wid: items.iter().map(|i| i.kind == Kind::Widget).collect(),
            vw,
            kk,
            flat,
            hover_lr: (0, 0),
        };
        // The hover region must contain the dock at every pointer position.
        let (mut lo, mut hi) = (x0, x0 + shelf_w);
        let mut px = x0 - geo.range;
        while px < x0 + shelf_w + geo.range {
            let (dx, scales) = geo.layout(px, 1.0);
            let grow: f32 = scales.iter().zip(&geo.vw).map(|(s, v)| (s - 1.0) * v).sum();
            lo = lo.min(dx);
            hi = hi.max(dx + shelf_w + grow);
            px += 2.0;
        }
        geo.hover_lr = ((lo - 4.0) as i32, (hi + 4.0) as i32);
        geo
    }

    fn center_abs(&self, i: usize) -> f32 {
        self.x0 + self.p + self.left[i] + self.width[i] / 2.0
    }

    /// The y a slot's scaling turns around: the bottom edge of an icon when the shelf sits
    /// at the bottom (icons grow up), the top edge when the panel is at the top (grow down).
    fn pivot_y(&self) -> f32 {
        if self.top { 0.0 } else { self.b }
    }

    /// The drawn vertical box of an item at scale `s`.
    fn item_box(&self, s: f32) -> (f32, f32) {
        if self.top {
            (self.y0 + self.p, self.y0 + self.p + self.b * s)
        } else {
            (self.y0 + self.p + self.b - self.b * s, self.y0 + self.p + self.b)
        }
    }

    /// Vertical band a pointer can be in and still touch the shelf: from the far edge of a
    /// fully magnified item out to the window's edge on the other side.
    fn band(&self) -> (f32, f32) {
        if self.top {
            (0.0, self.y0 + self.p + self.m + self.g)
        } else {
            (self.y0 + self.p + self.b - self.m - self.g, self.win_h)
        }
    }

    /// Is y within the shelf's strip (plus `slack` past its inner edge)?
    fn in_shelf_band(&self, y: f32, slack: f32) -> bool {
        if self.top {
            y < self.y0 + self.shelf_h + slack
        } else {
            y > self.y0 - slack
        }
    }

    /// Which item is under (x, y), measured in the unmagnified layout: hovering must not
    /// depend on where the animation happens to be this frame.
    fn hit_base(&self, x: f32, y: f32) -> Option<usize> {
        let (lo, hi) = self.band();
        if y < lo || y >= hi {
            return None;
        }
        for i in 0..self.left.len() {
            if !(self.icon[i] || self.wid[i]) {
                continue;
            }
            let l = self.x0 + self.p + self.left[i];
            if x >= l && x < l + self.width[i] {
                return Some(i);
            }
        }
        None
    }

    /// Rust mirror of the compositor expressions: (shelf x, per-item scale).
    fn layout(&self, px: f32, amount: f32) -> (f32, Vec<f32>) {
        let scales: Vec<f32> = (0..self.left.len())
            .map(|i| {
                if self.kk[i] <= 0.0 {
                    return 1.0;
                }
                let d = (((px - self.center_abs(i)).abs() - self.flat[i]).max(0.0) / self.range).min(1.0);
                1.0 + self.kk[i] * amount * ((d * PI).cos() + 1.0) / 2.0
            })
            .collect();
        let mut shift = 0.0;
        for i in 0..self.left.len() {
            let h = ((px - (self.x0 + self.p + self.left[i])) / self.width[i]).clamp(0.0, 1.0);
            shift += (scales[i] - 1.0) * self.vw[i] * h;
        }
        (self.x0 - shift, scales)
    }

    /// Centre x (window px) of item i when the pointer is at px, and its scale.
    fn slot_center(&self, px: f32, amount: f32, i: usize) -> (f32, f32) {
        let (dx, scales) = self.layout(px, amount);
        let acc: f32 = (0..i).map(|j| (scales[j] - 1.0) * self.vw[j]).sum();
        let l = dx + self.p + self.left[i] + acc + if self.icon[i] || self.wid[i] { self.g / 2.0 } else { 0.0 };
        (l + self.vw[i].max(self.b) * scales[i] / 2.0, scales[i])
    }

    /// Left and top edges (window px) of a widget item and its scale, pointer at px.
    fn item_rect(&self, px: f32, amount: f32, i: usize) -> (f32, f32, f32) {
        let (dx, scales) = self.layout(px, amount);
        let acc: f32 = (0..i).map(|j| (scales[j] - 1.0) * self.vw[j]).sum();
        let s = scales[i];
        (dx + self.p + self.left[i] + acc + self.g / 2.0, self.item_box(s).0, s)
    }

    /// Which icon is under (x, y)?
    fn hit(&self, x: f32, y: f32, amount: f32) -> Option<usize> {
        let (dx, scales) = self.layout(x, amount);
        let mut acc = 0.0;
        for i in 0..self.left.len() {
            let l = dx + self.p + self.left[i] + acc;
            let slot = self.width[i] + (scales[i] - 1.0) * self.vw[i];
            let (top, bottom) = self.item_box(scales[i]);
            if self.icon[i] && x >= l && x < l + slot && y >= top - self.g && y <= bottom {
                return Some(i);
            }
            if self.wid[i] && x >= l + self.g / 2.0 && x < l + slot - self.g / 2.0 && y >= top - 6.0 && y < bottom + 6.0 {
                return Some(i);
            }
            acc += (scales[i] - 1.0) * self.vw[i];
        }
        None
    }
}

/// Files being dragged over the dock from another app (OLE drag and drop).
struct FileDrag {
    files: Vec<String>,
    /// Every file is a program, shortcut or folder: always pin, never "open with".
    pinnable: bool,
    /// Alt is held: pin documents too.
    alt: bool,
    /// Slot before which a gap is open for pinning.
    insert: Option<usize>,
}

/// An icon being dragged.
struct Drag {
    /// Item index in `items` of the dragged icon.
    index: usize,
    /// Slot before which the gap is open (None: no gap, e.g. pulled off the dock).
    insert: Option<usize>,
    removing: bool,
    width: f32,
    /// Item the dragged app is held over (to make or extend a stack), and whether the hold completed.
    merge: Option<usize>,
    merge_ready: bool,
    /// Set when the icon came from Launchpad (not from the dock); `index` is then unused.
    ext: Option<String>,
    /// A widget dragged in from Launchpad: the pin to insert.
    ext_widget: Option<Pin>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrayOrigin {
    Dock,
    Popup,
    ControlCenter,
}

struct TrayPress {
    key: u64,
    origin: TrayOrigin,
    screen_x: i32,
    screen_y: i32,
}

enum TrayDrop {
    /// Insert beside another icon; `in_overflow` preserves/promotes the visibility group.
    Relative { stable_id: String, after: bool, in_overflow: bool },
    /// Move a visible strip icon into the overflow popup.
    Hide,
    /// Drag an overflow icon back to the visible strip.
    Promote,
}

struct TrayDrag {
    key: u64,
    stable_id: String,
    origin: TrayOrigin,
    target: Option<TrayDrop>,
}

/// The floating copy of an icon that follows the pointer while dragging.
struct Ghost {
    popup: crate::ui::Popup,
    body: ContainerVisual,
    size: i32,
}

#[derive(Clone, Copy, PartialEq, Default)]
struct IndState {
    running: bool,
    focused: bool,
    /// One of the app's windows flashed for attention and hasn't been looked at yet.
    attention: bool,
}

/// The green "needs attention" dot beside an app's running dot: a solid core with a
/// soft glow behind it that breathes while the app is waiting for you.
#[derive(Clone)]
struct Att {
    vis: ContainerVisual,
    glow: SpriteVisual,
    core: SpriteVisual,
    /// "Shift": how far the running dot moves left so the pair stays centred.
    props: CompositionPropertySet,
}

/// Everything the window procedure needs.
struct Dock {
    hwnd: HWND,
    opts: Options,
    started: Instant,
    cfg: Config,
    monitor: RECT,
    scale: f32,
    compositor: Compositor,
    _target: DesktopWindowTarget,
    gfx: Gfx,
    icons: IconCache,
    brushes: HashMap<String, CompositionSurfaceBrush>,
    /// Tray icon pictures by icon key: (picture generation, brush).
    tray_brushes: HashMap<u64, (u64, CompositionSurfaceBrush)>,
    /// Live tray icon metadata and the current strip/overflow partition.
    tray_items: Vec<crate::tray_strip::TrayIconView>,
    tray_bar_keys: Vec<u64>,
    tray_overflow_keys: Vec<u64>,
    tray_popup: crate::tray_strip::TrayPopup,
    mini_tray: crate::tray_mini::MiniTray,
    mini_press: Option<crate::tray_mini::Hit>,
    tray_hover: Option<(u64, bool)>,
    tray_tip_open: Option<u64>,
    tray_custom_popup: Option<u64>,
    tray_press: Option<TrayPress>,
    tray_drag: Option<TrayDrag>,
    tray_highlights: Vec<(u64, ShapeVisual)>,
    names: HashMap<String, String>,
    /// What glass shows: Windows' blurred desktop, plus any extra blur (see `update_glass`).
    backdrop: CompositionBrush,
    /// Windows' blurred desktop, unmodified.
    host_backdrop: CompositionBrush,
    ptr: CompositionPropertySet,
    root: ContainerVisual,
    dock: ContainerVisual,
    slots: Vec<ContainerVisual>,
    /// Running totals for the shelf shift (one property set per slot, chained).
    accs: Vec<CompositionPropertySet>,
    tiles: Vec<Option<Visual>>,
    inds: Vec<Option<SpriteVisual>>,
    ind_state: Vec<IndState>,
    /// Attention dots, one per item (None for non-apps).
    atts: Vec<Option<Att>>,
    /// Windows (HWND values) that asked for attention (FlashWindow) and haven't been focused.
    attention: std::collections::HashSet<isize>,
    label: ContainerVisual,
    label_geom: CompositionRoundedRectangleGeometry,
    label_text: SpriteVisual,
    items: Vec<Item>,
    geo: Geo,
    // windows
    tracker: Tracker,
    windows: Vec<Win>,
    running_order: Vec<String>,
    fg: HWND,
    /// Set while the panel is hidden because a full-screen app is in front.
    hidden_for_fullscreen: bool,
    /// Windows changed light/dark: repaint everything on the next tick.
    retheme_pending: bool,
    resolved: HashMap<String, Option<String>>,
    // interaction
    hovering: bool,
    hover: Option<usize>,
    pressed: Option<usize>,
    tucked: bool,
    menu_open: bool,
    menu: Menu,
    /// Drag & drop in the dock (reordering, removing, pinning).
    drag: Option<Drag>,
    press: Option<(usize, f32, f32)>,
    ghost: Option<Ghost>,
    /// Live window previews above a hovered running app.
    preview: Preview,
    preview_pending: Option<usize>,
    /// Stack pop-up (grid of a stack's apps).
    stack_view: StackView,
    /// Stack (pin index) the open menu is about.
    menu_stack: Option<usize>,
    /// Launchpad and the data it shows (fetched by the services worker).
    launchpad: Launchpad,
    /// The control centre: one glass drop-down above the panel, split into sections.
    control_center: crate::control_center::ControlCenter,
    services: Services,
    lp_apps: Vec<AppEntry>,
    control: ControlState,
    stats: Option<Stats>,
    weather: Option<Weather>,
    files: Vec<FileHit>,
    /// What the user picked in Launchpad search (search-history.json next to config.json).
    history: laminar_sys::history::History,
    files_gen: crate::launchpad_input::SearchEpoch,
    icons_requested: std::collections::HashSet<String>,
    user_name: String,
    lp_drag: Option<Act>,
    /// Dock icon highlighted while files are dragged over it.
    drop_target: Option<usize>,
    file_drag: Option<FileDrag>,
    /// Visible Wi-Fi networks for Launchpad (None until asked), or the Windows error code.
    wifi: Option<std::result::Result<Vec<laminar_sys::radios::WifiNetwork>, u32>>,
    /// Windows Update status and when it was last asked for.
    update: Option<laminar_sys::update::UpdateStatus>,
    update_asked: Option<Instant>,
    /// Keeps the OLE drop target alive.
    _drop: Option<windows::Win32::System::Ole::IDropTarget>,
    _preview_drop: Option<windows::Win32::System::Ole::IDropTarget>,
    /// During a drag: the running app under the pointer (its tiles open after a pause).
    drag_under: Option<usize>,
    /// During a drag: the window tile under the pointer (it comes forward after a pause).
    drag_switch: Option<HWND>,
    /// The open previews were opened by a drag (they stay up while it moves to them).
    drag_preview: bool,
    /// The Laminar version downloaded and ready to install, if any.
    app_update: Option<String>,
    /// The user asked to check (so report "up to date" too, and ignore snooze/skip).
    app_update_manual: bool,
    /// App a Launchpad right-click menu is about.
    lp_menu_app: Option<String>,
    /// An app tile pressed in Launchpad (id, x, y) — becomes a drag into the dock if moved.
    lp_press: Option<(String, f32, f32)>,
    /// What the open menu is about: the app item (if any) and its windows.
    menu_ctx: Option<(Option<Item>, Vec<Win>)>,
    // system integration (off in --measure / --demo-hover)
    integrate: bool,
    /// This is a helper dock for an extra display (no taskbar takeover / Windows key).
    helper: bool,
    helpers: crate::displays::Helpers,
    /// Which edge holds our appbar reservation: `Some(true)` = top, `None` = not reserved.
    /// Remembered per edge because moving the panel needs a remove-and-register, not a
    /// second ABM_SETPOS — the old strip stayed reserved otherwise.
    appbar_edge: Option<bool>,
    appbar_rect: Option<crate::appbar_policy::Position>,
    tray_original: Option<u32>,
    taskbar_hidden: bool,
    taskbar_watchdog: Option<std::process::Child>,
    // measurement
    measure_step: u32,
    m_cpu0: f64,
    m_t0: Instant,
    report: Vec<(String, String)>,
    sweep_t0: Instant,
    sweep_moves: u32,
    last_scan_ms: f64,
    last_refresh_ms: f64,
    /// Widgets: live state and feeds, and the dock's painted widgets by item index.
    hub: crate::widgets::Hub,
    wviews: HashMap<usize, WView>,
    /// Pointer position (window px) at the last move or click.
    ptr_xy: (f32, f32),
    /// Where the magnification peak sits: the centre of the slot under the pointer.
    anchor_x: f32,
    /// Pin index of the widget a dock menu is about.
    menu_widget: Option<usize>,
    /// Launchpad widget (list index) a menu is about.
    menu_lpw: Option<crate::launchpad::WPos>,
    /// A Launchpad widget card pressed (index, x, y): a click, or a reorder drag if it moves.
    lpw_press: Option<(crate::launchpad::DragSrc, f32, f32)>,
    /// Ticks of the widget timer (seconds).
    wtick: u64,
    /// The Start button's four panes, and whether they're spread (hovered).
    /// The desktop clock card, hosted by `widgets.exe` in its own process.
    clock_host: Option<std::process::Child>,
    /// When the card may be started again after it died (never on a tight loop).
    clock_retry_at: Instant,
    /// The desktop load card (processor, memory, disk), also hosted by `widgets.exe`.
    system_host: Option<std::process::Child>,
    /// When the load card may be started again after it died.
    system_retry_at: Instant,
    /// The y the panel's bottom edge sits on. The monitor's bottom edge, unless Windows'
    /// own taskbar is on screen: then it is the top of that taskbar, so the panel goes
    /// above it instead of underneath.
    floor_y: i32,
    /// The panel hangs from the top edge of the monitor, because Explorer's taskbar keeps
    /// the bottom one. Everything that pops out of the panel follows this.
    panel_top: bool,
    /// Whether the panel window currently sits in the topmost band (`WS_EX_TOPMOST`). A
    /// bottom panel belongs there; a top panel must be taken out of it, and Windows keeps a
    /// window topmost until it is told otherwise.
    band_topmost: bool,
    /// Whether the panel has been put into its band at least once (see `place_window`).
    z_placed: bool,
    start_panes: Vec<StartPane>,
    start_open: bool,
}

/// The Start mark's hover effect: a property set whose `Sheen` value moves the reflection.
struct StartPane {
    fx: CompositionPropertySet,
    b: f32,
}

/// The Start button: the Windows mark itself, four panes and nothing behind them, in the
/// logo's blue (one gradient running across all four, so they read as a single pane of
/// glass). Vector shapes and brushes only, so it stays crisp at any magnification. Each
/// pane carries a hidden reflection band that `start_hover` sweeps across.
fn start_tile(c: &Compositor, b: f32) -> Result<(ContainerVisual, Vec<StartPane>)> {
    let tile = c.CreateContainerVisual()?;
    tile.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
    let mark = (b * 0.74).round();
    let gap = (b * 0.04).max(2.0).round();
    let pane = ((mark - gap) / 2.0).round();
    let x0 = ((b - (2.0 * pane + gap)) / 2.0).round();
    let radius = (pane * 0.08).max(1.0);
    let fx = c.CreatePropertySet()?;
    fx.InsertScalar(&hs("Sheen"), -b)?;
    let fx_obj: CompositionObject = fx.cast()?;
    let col = |r: u8, g: u8, bl: u8, a: u8| Color { A: a, R: r, G: g, B: bl };
    for (cx, cy) in [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let (px, py) = (x0 + cx * (pane + gap), x0 + cy * (pane + gap));
        let holder = c.CreateContainerVisual()?;
        holder.SetSize(v2(pane, pane))?;
        holder.SetOffset(v3(px, py, 0.0))?;
        let geo = c.CreateRoundedRectangleGeometry()?;
        geo.SetSize(v2(pane, pane))?;
        geo.SetCornerRadius(v2(radius, radius))?;
        holder.SetClip(&c.CreateGeometricClipWithGeometry(&geo)?)?;
        // The fill: a light-to-deep blue diagonal measured across the whole mark.
        let fill = c.CreateSpriteVisual()?;
        fill.SetSize(v2(pane, pane))?;
        let grad = c.CreateLinearGradientBrush()?;
        grad.SetMappingMode(CompositionMappingMode::Absolute)?;
        grad.SetStartPoint(v2(x0 - px, x0 - py))?;
        grad.SetEndPoint(v2(x0 + mark - px, x0 + mark - py))?;
        grad.ColorStops()?.Append(&c.CreateColorGradientStopWithOffsetAndColor(0.0, col(0x7A, 0xD3, 0xFF, 255))?)?;
        grad.ColorStops()?.Append(&c.CreateColorGradientStopWithOffsetAndColor(0.55, col(0x1E, 0x90, 0xF0, 255))?)?;
        grad.ColorStops()?.Append(&c.CreateColorGradientStopWithOffsetAndColor(1.0, col(0x0A, 0x5F, 0xC8, 255))?)?;
        fill.SetBrush(&grad)?;
        holder.Children()?.InsertAtTop(&fill)?;
        // A faint top edge highlight, like light catching glass.
        let edge = c.CreateSpriteVisual()?;
        edge.SetSize(v2(pane, (pane * 0.5).round()))?;
        let eg = c.CreateLinearGradientBrush()?;
        eg.SetStartPoint(v2(0.0, 0.0))?;
        eg.SetEndPoint(v2(0.0, 1.0))?;
        eg.ColorStops()?.Append(&c.CreateColorGradientStopWithOffsetAndColor(0.0, col(255, 255, 255, 46))?)?;
        eg.ColorStops()?.Append(&c.CreateColorGradientStopWithOffsetAndColor(1.0, col(255, 255, 255, 0))?)?;
        edge.SetBrush(&eg)?;
        holder.Children()?.InsertAtTop(&edge)?;
        // The reflection: a soft diagonal band, positioned from the shared Sheen value so
        // it sweeps across all four panes as one.
        let band = c.CreateSpriteVisual()?;
        band.SetSize(v2(2.0 * b, 2.0 * b))?;
        let bg = c.CreateLinearGradientBrush()?;
        bg.SetStartPoint(v2(0.0, 0.0))?;
        bg.SetEndPoint(v2(1.0, 1.0))?;
        for (o, a) in [(0.0, 0u8), (0.43, 0), (0.5, 120), (0.57, 0), (1.0, 0)] {
            bg.ColorStops()?.Append(&c.CreateColorGradientStopWithOffsetAndColor(o, col(255, 255, 255, a))?)?;
        }
        band.SetBrush(&bg)?;
        let e = c.CreateExpressionAnimationWithExpression(&hs(&format!("Vector3(fx.Sheen - {b} - {px}, fx.Sheen - {b} - {py}, 0)")))?;
        e.SetReferenceParameter(&hs("fx"), &fx_obj)?;
        band.StartAnimation(&hs("Offset"), &e)?;
        holder.Children()?.InsertAtTop(&band)?;
        tile.Children()?.InsertAtTop(&holder)?;
    }
    Ok((tile, vec![StartPane { fx, b }]))
}

/// A widget drawn in the dock.
struct WView {
    def: &'static crate::widgets::Def,
    surface: CompositionDrawingSurface,
    tile: ContainerVisual,
    painted: crate::widgets::Painted,
    w: f32,
    h: f32,
    /// Surface resolution relative to w×h (the magnified size).
    res: f32,
}

thread_local! {
    static DOCK: RefCell<Option<Dock>> = const { RefCell::new(None) };
}

/// Open the panel's settings page. One action behind three ways in: the «Настройки» button,
/// `AuraDock.exe --settings` at startup, and the message a second `AuraDock.exe --settings`
/// posts to the panel that is already running.
fn dock_show_settings() {
    with_dock(|d| {
        d.open_launchpad();
        d.lp_act(Act::OpenSettings);
    });
}

/// `AuraDock.exe --settings` while a panel is already running: the second copy cannot live
/// (one panel per display), so it hands the request to the first.
pub(crate) fn show_settings_of_running_panel() {
    unsafe {
        let Ok(window) = FindWindowW(w!("AuraExplorerDock"), windows::core::PCWSTR::null()) else {
            return;
        };
        let _ = PostMessageW(Some(window), WM_APP_SETTINGS, WPARAM(0), LPARAM(0));
    }
}

fn with_dock(f: impl FnOnce(&mut Dock)) {
    DOCK.with(|d| {
        if let Ok(mut g) = d.try_borrow_mut() {
            if let Some(dock) = g.as_mut() {
                f(dock);
            }
        }
    });
}

/// Like with_dock, returning a value (None if the dock is busy, e.g. a re-entrant call).
fn with_dock_ret<T>(f: impl FnOnce(&mut Dock) -> T) -> Option<T> {
    DOCK.with(|d| d.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn hs(s: &str) -> HSTRING {
    HSTRING::from(s)
}
fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}
fn v3(x: f32, y: f32, z: f32) -> Vector3 {
    Vector3 { X: x, Y: y, Z: z }
}
fn ms(n: i64) -> TimeSpan {
    TimeSpan { Duration: n * 10_000 }
}

/// Debug log to stderr when LAMINAR_TRACE is set, with milliseconds since start.
macro_rules! trace {
    ($($arg:tt)*) => {
        if crate::dock::tracing() {
            eprintln!("[{:>7.1} ms] {}", crate::dock::trace_clock(), format!($($arg)*));
        }
    };
}
pub(crate) fn tracing() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("LAMINAR_TRACE").is_ok())
}
/// Widget connector messages go to the trace log only (they can mention service names, never keys).
pub(crate) fn trace_widget(msg: &str) {
    if tracing() {
        eprintln!("[{:>7.1} ms] widgets: {msg}", trace_clock());
    }
}
pub(crate) fn trace_clock() -> f64 {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// Durations of the auto-hide slide out and back in.
const TUCK_MS: u32 = 300;
const SHOW_MS: u32 = 460;

/// Smooth start and finish (cubic), for motion that leaves the screen.
fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Testing aid: LAMINAR_SLOWMO=20 makes dock motion 20x slower so it can be inspected.
fn slowmo() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("LAMINAR_SLOWMO").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0f32).max(0.01))
}

/// Windows' "Animation effects" setting (Settings > Accessibility > Visual effects).
fn animations_enabled() -> bool {
    if testctl() {
        return false;
    }
    let mut on = windows::core::BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    on.as_bool()
}

/// False while a full-screen game, video or presentation is in front (don't peek over it).
fn quiet_ok() -> bool {
    use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_ACCEPTS_NOTIFICATIONS};
    unsafe { SHQueryUserNotificationState().map(|s| s == QUNS_ACCEPTS_NOTIFICATIONS).unwrap_or(true) }
}

/// Damped-spring progress at time t (0..1), the same curve the prototype uses.
fn spring(t: f32, bounce: f32) -> f32 {
    let zeta = (1.0 - bounce).max(0.36);
    let w = 6.2 / zeta;
    if zeta < 1.0 {
        let wd = w * (1.0 - zeta * zeta).sqrt();
        1.0 - (-zeta * w * t).exp() * ((wd * t).cos() + (zeta * w / wd) * (wd * t).sin())
    } else {
        1.0 - (-w * t).exp() * (1.0 + w * t)
    }
}

impl Dock {
    // ---------------------------------------------------------------- helpers

    fn expr(&self, text: &str, refs: &[(&str, &CompositionObject)]) -> Result<ExpressionAnimation> {
        let e = self.compositor.CreateExpressionAnimationWithExpression(&hs(text))?;
        e.SetReferenceParameter(&hs("ptr"), &self.ptr)?;
        for (name, obj) in refs {
            e.SetReferenceParameter(&hs(name), *obj)?;
        }
        Ok(e)
    }

    /// Glass = live blurred desktop (HostBackdropBrush) + tint, clipped to a rounded rectangle.
    fn glass(&self, tint: f32, radius: f32) -> Result<(ContainerVisual, CompositionRoundedRectangleGeometry)> {
        let c = &self.compositor;
        let cont = c.CreateContainerVisual()?;
        let blur = c.CreateSpriteVisual()?;
        blur.SetBrush(&self.backdrop)?;
        blur.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        let t = c.CreateSpriteVisual()?;
        // Light theme: pale glass, a little denser so dark icons and dots read on any wallpaper.
        let tint_color = if crate::ui::theme::light() {
            Color { A: ((tint + 0.12).min(0.92) * 255.0) as u8, R: 244, G: 244, B: 248 }
        } else {
            Color { A: (tint * 255.0) as u8, R: 30, G: 30, B: 38 }
        };
        t.SetBrush(&c.CreateColorBrushWithColor(tint_color)?)?;
        t.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        cont.Children()?.InsertAtTop(&blur)?;
        cont.Children()?.InsertAtTop(&t)?;
        let geom = c.CreateRoundedRectangleGeometry()?;
        geom.SetCornerRadius(v2(radius, radius))?;
        cont.SetClip(&c.CreateGeometricClipWithGeometry(&geom)?)?;
        Ok((cont, geom))
    }

    fn pin_path(&mut self, pin: &str) -> Option<String> {
        if !self.resolved.contains_key(pin) {
            let r = self.icons.target(pin); // cached on disk; asks the shell only for new pins
            self.resolved.insert(pin.to_string(), r);
        }
        self.resolved[pin].clone()
    }

    fn pin_matches(&mut self, pin: &str, w: &Win) -> bool {
        let p = pin.to_lowercase();
        if p == w.app || p == w.exe {
            return true;
        }
        // The target comes back from the shell, so its case is whatever the shell felt like;
        // the window's path is already lower-cased. Comparing the raw strings missed the
        // match and the same program then sat on the panel twice (once pinned, once running).
        self.pin_path(pin).map(|r| r.to_lowercase() == w.exe).unwrap_or(false)
    }

    /// Every identity one running app answers to: its id and the executables of its windows.
    fn app_keys(&self, app: &str) -> Vec<String> {
        let mut keys = vec![app.to_lowercase()];
        for w in self.windows.iter().filter(|w| w.app == app) {
            if !w.exe.is_empty() && !keys.contains(&w.exe) {
                keys.push(w.exe.clone());
            }
        }
        keys
    }

    /// Claim the identities a pin stands for — its own id and the program it points at — so
    /// a running window of the same program doesn't earn a second icon.
    fn claim_pin(&mut self, pin: &str, taken: &mut std::collections::HashSet<String>) {
        taken.insert(pin.to_lowercase());
        if let Some(p) = self.pin_path(pin) {
            taken.insert(p.to_lowercase());
        }
    }

    /// Windows belonging to a dock item, front to back.
    fn item_windows(&mut self, it: &Item) -> Vec<Win> {
        if it.kind != Kind::App {
            return Vec::new();
        }
        let wins = self.windows.clone();
        if it.pinned {
            wins.into_iter().filter(|w| self.pin_matches(&it.key, w)).collect()
        } else {
            wins.into_iter().filter(|w| w.app == it.key).collect()
        }
    }

    // ---------------------------------------------------------------- model

    fn compute_items(&mut self) -> Vec<Item> {
        self.tray_items = self.collect_tray_items();
        let (tray_bar, tray_overflow) = crate::tray_strip::partition(
            &self.tray_items.iter().filter(|icon| crate::tray_compact::show_icon(&icon.stable_id, icon.app_hidden)).cloned().collect::<Vec<_>>(),
            &self.cfg.tray_order,
            &self.cfg.tray_hidden,
            crate::tray_mini_layout::icon_capacity(self.monitor.right - self.monitor.left, self.scale),
        );
        self.tray_bar_keys = tray_bar.iter().map(|i| i.key).collect();
        self.tray_overflow_keys = tray_overflow.iter().map(|i| i.key).collect();
        let mut v = vec![Item { kind: Kind::Launch, key: "::launch".into(), pinned: true }];
        let pins = self.cfg.pins.clone();
        // The control centre is a pinned widget, but it belongs at the right-hand end of
        // the panel (where the Recycle Bin used to sit), after the running apps.
        // The control center now has a dedicated button in the right-hand mini tray.
        // Identities already on the panel. One program, one icon: two pins for the same app,
        // a stack member that is also running, a window that reports its AppUserModelID while
        // its twin reports the executable path — all of those used to draw twice.
        let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut pin_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (n, p) in pins.iter().enumerate() {
            match p {
                Pin::Id(id) if id == SEPARATOR => v.push(Item { kind: Kind::Separator, key: format!("::sep{n}"), pinned: true }),
                Pin::Id(id) => {
                    // The same program pinned twice: keep the first, leave the config alone.
                    if !pin_ids.insert(id.to_lowercase()) {
                        continue;
                    }
                    self.claim_pin(id, &mut taken);
                    v.push(Item { kind: Kind::App, key: id.clone(), pinned: true });
                }
                Pin::Stack { apps, .. } => {
                    for a in apps {
                        self.claim_pin(a, &mut taken);
                    }
                    v.push(Item { kind: Kind::Stack, key: format!("::stack{n}"), pinned: true });
                }
                Pin::Widget { widget, size } => {
                    // Old separately pinned tray widgets must not duplicate the tray on the dock.
                    if widget == "background-apps" { continue; }
                    let sz = size.or_else(|| crate::widgets::catalog::get(widget).map(|d| d.size)).unwrap_or_default();
                    let code = match sz {
                        crate::widgets::Size::Small => "s",
                        crate::widgets::Size::Medium => "m",
                        crate::widgets::Size::Large => "l",
                    };
                    let item = Item { kind: Kind::Widget, key: format!("::widget{n}:{widget}:{code}"), pinned: true };
                    if widget != CONTROL_CENTER { v.push(item); }
                }
            }
        }
        // Running apps that aren't already on the panel, in the order they first appeared.
        // An app is identified by the program, not by the id Windows happens to report for
        // one of its windows: the same program can show up under an AppUserModelID and under
        // its executable path at once, and both must land on the same icon.
        let order = self.running_order.clone();
        let mut extra = Vec::new();
        for app in order {
            let keys = self.app_keys(&app);
            if keys.iter().any(|k| taken.contains(k)) {
                continue;
            }
            taken.extend(keys);
            extra.push(Item { kind: Kind::App, key: app, pinned: false });
        }
        if !extra.is_empty() {
            v.push(Item { kind: Kind::Separator, key: "::sep-run".into(), pinned: false });
            v.extend(extra);
        }
        v.push(Item { kind: Kind::Separator, key: "::sep-end".into(), pinned: false });
        // All notification icons live in Control Center, never as large dock tiles.
        if self.update_icon_visible() {
            v.push(Item { kind: Kind::Update, key: "::update".into(), pinned: true });
        }
        v
    }

    /// Show the update icon: a version is ready, and the user hasn't snoozed or skipped it
    /// (asking to check by hand overrides both).
    fn update_icon_visible(&self) -> bool {
        let Some(v) = &self.app_update else { return false };
        if self.app_update_manual {
            return true;
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        *v != self.cfg.update_skip && now >= self.cfg.update_snooze_until
    }

    /// (pin index, name, app ids) of a stack item.
    fn stack_of(&self, key: &str) -> Option<(usize, String, Vec<String>)> {
        let n: usize = key.strip_prefix("::stack")?.parse().ok()?;
        match self.cfg.pins.get(n)? {
            Pin::Stack { stack, apps } => Some((n, stack.clone(), apps.clone())),
            _ => None,
        }
    }

    /// Icon brush for an item, loading it (from the cache, or the shell once) if needed.
    fn ensure_brush(&mut self, it: &Item) -> bool {
        if it.kind == Kind::Stack {
            if let Some((_, _, apps)) = self.stack_of(&it.key) {
                for a in apps {
                    self.ensure_brush(&Item { kind: Kind::App, key: a, pinned: true });
                }
            }
            return true;
        }
        if it.kind == Kind::TrayOverflow {
            return true;
        }
        if matches!(it.kind, Kind::Tray(_)) {
            return self.brushes.contains_key(&it.key);
        }
        if matches!(it.kind, Kind::Separator | Kind::Widget) || self.brushes.contains_key(&it.key) {
            return true;
        }
        let px = (self.cfg.magnified_size.max(self.cfg.icon_size) * self.scale).ceil();
        let surface = match it.kind {
            // The Start button is drawn with vector shapes in `rebuild` (see `start_tile`).
            Kind::Launch => return true,
            Kind::Update => {
                self.names.insert(it.key.clone(), "Update AuraDock".into());
                self.gfx.update_surface(px).ok()
            }
            _ => {
                let exe = self.windows.iter().find(|w| w.app == it.key).map(|w| w.exe.clone());
                let got = self.icons.get(&self.gfx.wic, &it.key).or_else(|| exe.and_then(|e| self.icons.get(&self.gfx.wic, &e)));
                got.and_then(|(n, src)| {
                    self.names.insert(it.key.clone(), n);
                    self.gfx.icon_surface(&src, px).ok()
                })
            }
        };
        let Some(surface) = surface else { return false };
        let Ok(brush) = self.compositor.CreateSurfaceBrushWithSurface(&surface) else { return false };
        let _ = brush.SetStretch(CompositionStretch::Fill);
        let _ = brush.SetBitmapInterpolationMode(CompositionBitmapInterpolationMode::Linear);
        self.brushes.insert(it.key.clone(), brush);
        true
    }

    /// Recompute items; rebuild the visual tree only if they changed.
    fn refresh(&mut self, force: bool) {
        let old_tray: Vec<(u64, String, String, bool, bool)> = self.tray_items.iter()
            .map(|icon| (icon.key, icon.stable_id.clone(), icon.tip.clone(), icon.app_hidden, icon.standard_tip))
            .collect();
        let old_bar_keys = self.tray_bar_keys.clone();
        let old_overflow_keys = self.tray_overflow_keys.clone();
        let candidates = self.compute_items();
        let tray_changed = old_tray != self.tray_items.iter()
            .map(|icon| (icon.key, icon.stable_id.clone(), icon.tip.clone(), icon.app_hidden, icon.standard_tip))
            .collect::<Vec<_>>()
            || old_bar_keys != self.tray_bar_keys
            || old_overflow_keys != self.tray_overflow_keys;
        if tray_changed && self.tray_hover.is_some() {
            self.set_tray_hover(None, false);
        }
        let ok: Vec<bool> = candidates.iter().map(|it| self.ensure_brush(it)).collect();
        // Drop apps whose icon couldn't load (e.g. uninstalled); no leading/trailing/doubled separators.
        let mut clean: Vec<Item> = Vec::new();
        for (it, ok) in candidates.into_iter().zip(ok) {
            if !ok {
                continue;
            }
            if it.kind == Kind::Separator && clean.last().map(|l| l.kind == Kind::Separator || l.kind == Kind::Launch).unwrap_or(true) {
                continue;
            }
            clean.push(it);
        }
        self.icons.flush();
        let visual_changed = force || clean != self.items;
        if visual_changed {
            self.items = clean;
            if let Err(e) = self.rebuild() {
                eprintln!("rebuild failed: {e:?}");
            }
        }
        self.update_indicators();
        if self.is_main_dock() && !self.hidden_for_fullscreen && (force || tray_changed || !self.mini_tray.visible) {
            self.refresh_mini_tray();
        }
        if self.control_center.open {
            let anchor = self.cc_anchor();
            self.control_center.reposition(self.monitor, anchor, false);
        }
        if self.tray_popup.open && (visual_changed || tray_changed) {
            self.show_tray_popup();
        }
    }

    // ---------------------------------------------------------------- visual tree

    fn rebuild(&mut self) -> Result<()> {
        trace!("rebuild: {} items on monitor ({}, {})-({}, {}) scale {}", self.items.len(), self.monitor.left, self.monitor.top, self.monitor.right, self.monitor.bottom, self.scale);
        self.geo = Geo::new(&self.items, &self.cfg, self.scale, (self.monitor.right - self.monitor.left) as f32, self.panel_top);
        self.place_window()?;
        let c = self.compositor.clone();
        let geo_k = self.geo.m / self.geo.b - 1.0;

        self.dock.Children()?.RemoveAll()?;
        self.slots.clear();
        self.tiles.clear();
        self.inds.clear();
        self.ind_state.clear();
        self.atts.clear();
        self.tray_highlights.clear();
        self.wviews.clear();
        self.hover = None;
        let _ = self.show_label(None);

        self.root.SetSize(v2(self.geo.win_w, self.geo.win_h))?;
        if !self.hovering {
            self.anchor_x = self.geo.x0 + self.geo.shelf_w / 2.0;
            self.ptr.InsertScalar(&hs("PointerX"), self.anchor_x)?;
        }

        let geo = &self.geo;
        let mut widget_tiles: Vec<(usize, &'static crate::widgets::Def, ContainerVisual, f32)> = Vec::new();
        let mut start_panes: Vec<StartPane> = Vec::new();
        // Slots (one container per item), created first so expressions can reference them.
        for (idx, it) in self.items.iter().enumerate() {
            let slot = c.CreateContainerVisual()?;
            if it.kind == Kind::Launch {
                slot.SetSize(v2(geo.b, geo.b))?;
                slot.SetCenterPoint(v3(0.0, geo.pivot_y(), 0.0))?;
                let (tile, panes) = start_tile(&c, geo.b)?;
                slot.Children()?.InsertAtTop(&tile)?;
                self.tiles.push(Some(tile.cast()?));
                start_panes = panes;
            } else if let (Kind::Widget, Some(def)) = (it.kind, widget_of(&it.key)) {
                let ww = crate::widgets::dock_width(def, widget_size(&it.key), geo.b);
                slot.SetSize(v2(ww, geo.b))?;
                // Grow up and to the right from the bottom-left corner, like an icon.
                slot.SetCenterPoint(v3(0.0, geo.pivot_y(), 0.0))?;
                let tile = c.CreateContainerVisual()?;
                tile.SetSize(v2(ww, geo.b))?;
                slot.Children()?.InsertAtTop(&tile)?;
                self.tiles.push(Some(tile.cast()?));
                widget_tiles.push((idx, def, tile, ww));
            } else if it.kind == Kind::Stack {
                slot.SetSize(v2(geo.b, geo.b))?;
                slot.SetCenterPoint(v3(0.0, geo.pivot_y(), 0.0))?;
                // A frosted tile with up to four mini icons.
                let tile = c.CreateContainerVisual()?;
                tile.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
                let (bg, _, bg_shape) = crate::ui::rounded(&c, geo.b, geo.b, geo.b * 0.24, crate::ui::theme::ink(0.16))?;
                bg_shape.SetStrokeBrush(&c.CreateColorBrushWithColor(crate::ui::theme::ink(0.28))?)?;
                bg_shape.SetStrokeThickness(1.0)?;
                tile.Children()?.InsertAtTop(&bg)?;
                let apps = self.stack_of(&it.key).map(|v| v.2).unwrap_or_default();
                let pad = geo.b * 0.15;
                let gap = geo.b * 0.07;
                let mini = (geo.b - 2.0 * pad - gap) / 2.0;
                for (k, a) in apps.iter().take(4).enumerate() {
                    if let Some(b) = self.brushes.get(a) {
                        let sp = c.CreateSpriteVisual()?;
                        sp.SetBrush(b)?;
                        sp.SetSize(v2(mini, mini))?;
                        sp.SetOffset(v3(pad + (k % 2) as f32 * (mini + gap), pad + (k / 2) as f32 * (mini + gap), 0.0))?;
                        tile.Children()?.InsertAtTop(&sp)?;
                    }
                }
                slot.Children()?.InsertAtTop(&tile)?;
                self.tiles.push(Some(tile.cast()?));
            } else if let Kind::Tray(key) = it.kind {
                slot.SetSize(v2(geo.b, geo.b))?;
                slot.SetCenterPoint(v3(0.0, geo.pivot_y(), 0.0))?;
                let bg_size = geo.b * 0.86;
                let (bg, _, _) = crate::ui::rounded(&c, bg_size, bg_size, 7.0 * self.scale, crate::ui::theme::fill_2())?;
                bg.SetOffset(v3((geo.b - bg_size) / 2.0, (geo.b - bg_size) / 2.0, 0.0))?;
                bg.SetOpacity(0.0)?;
                slot.Children()?.InsertAtBottom(&bg)?;
                self.tray_highlights.push((key, bg));
                let tile = c.CreateSpriteVisual()?;
                tile.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
                if let Some(brush) = self.brushes.get(&it.key) { tile.SetBrush(brush)?; }
                slot.Children()?.InsertAtTop(&tile)?;
                self.tiles.push(Some(tile.cast()?));
            } else if it.kind == Kind::TrayOverflow {
                slot.SetSize(v2(geo.b, geo.b))?;
                slot.SetCenterPoint(v3(0.0, geo.pivot_y(), 0.0))?;
                let bg_size = geo.b * 0.86;
                let (bg, _, _) = crate::ui::rounded(&c, bg_size, bg_size, 7.0 * self.scale, crate::ui::theme::fill_2())?;
                bg.SetOffset(v3((geo.b - bg_size) / 2.0, (geo.b - bg_size) / 2.0, 0.0))?;
                bg.SetOpacity(0.0)?;
                slot.Children()?.InsertAtBottom(&bg)?;
                self.tray_highlights.push((u64::MAX, bg));
                let tile = c.CreateContainerVisual()?;
                tile.SetSize(v2(geo.b, geo.b))?;
                let dot = (3.0 * self.scale).round().max(2.0);
                let (r, g, b, a) = crate::ui::theme::text_2();
                let dot_color = crate::ui::rgba(
                    (r * 255.0).round() as u8,
                    (g * 255.0).round() as u8,
                    (b * 255.0).round() as u8,
                    a,
                );
                let center = geo.b / 2.0;
                for dx in [-1.0f32, 0.0, 1.0] {
                    let (shape, _, _) = crate::ui::rounded(&c, dot, dot, dot / 2.0, dot_color)?;
                    shape.SetOffset(v3(center + dx * dot * 2.1 - dot / 2.0, center - dot / 2.0, 0.0))?;
                    tile.Children()?.InsertAtTop(&shape)?;
                }
                slot.Children()?.InsertAtTop(&tile)?;
                self.tiles.push(Some(tile.cast()?));
            } else if it.is_icon() {
                slot.SetSize(v2(geo.b, geo.b))?;
                slot.SetCenterPoint(v3(0.0, geo.pivot_y(), 0.0))?;
                let tile = c.CreateSpriteVisual()?;
                tile.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
                tile.SetBrush(&self.brushes[&it.key])?;
                slot.Children()?.InsertAtTop(&tile)?;
                self.tiles.push(Some(tile.cast()?));
            } else {
                let line = c.CreateSpriteVisual()?;
                line.SetBrush(&c.CreateColorBrushWithColor(crate::ui::theme::ink(0.34))?)?;
                let lw = self.scale.round().max(1.0);
                line.SetSize(v2(lw, geo.b * 0.72))?;
                line.SetOffset(v3(((geo.sep_w - lw) / 2.0).round(), geo.b * 0.14, 0.0))?;
                slot.Children()?.InsertAtTop(&line)?;
                self.tiles.push(None);
            }
            self.slots.push(slot);
        }

        // Expressions are chained (each refers only to its left neighbour) so they stay short
        // however many icons there are: Windows rejects expression strings that are too long.
        let n = self.slots.len();
        let base_x = |i: usize| geo.p + geo.left[i] + if geo.icon[i] || geo.wid[i] { geo.g / 2.0 } else { 0.0 };
        let grow_of = |name: &str, i: usize| {
            if geo.kk[i] > 0.0 { format!("(({name}.Scale.X - 1) * {w})", w = geo.vw[i]) } else { "0".to_string() }
        };

        // Each icon's scale: cosine falloff from the pointer, blended by Amount.
        let _ = geo_k;
        for (i, slot) in self.slots.iter().enumerate() {
            if geo.kk[i] <= 0.0 {
                continue;
            }
            // Widgets count the pointer as "on" them anywhere across their width.
            let s = format!(
                "1 + {k} * ptr.Amount * ((Cos(Min(Max(Abs(ptr.PointerX - {c}) - {f}, 0) / {r}, 1) * 3.14159265) + 1) / 2)",
                k = geo.kk[i],
                c = geo.center_abs(i),
                f = geo.flat[i],
                r = geo.range
            );
            slot.StartAnimation(&hs("Scale"), &self.expr(&format!("Vector3({s}, {s}, 1)"), &[])?)?;
        }
        // Per-slot animatable values (one property set per slot):
        //   Acc      running total of magnification growth left of the pointer (shelf shift)
        //   Gap      space opened before this slot while dragging (where an icon will land)
        //   Collapse space closed before this slot (the dragged icon's old place)
        self.accs.clear();
        for _ in 0..n {
            let acc = c.CreatePropertySet()?;
            acc.InsertScalar(&hs("Acc"), 0.0)?;
            acc.InsertScalar(&hs("Gap"), 0.0)?;
            acc.InsertScalar(&hs("Collapse"), 0.0)?;
            self.accs.push(acc);
        }
        // Each slot's position: left neighbour's position + the rest gap + the neighbour's growth
        // (+ drag gap − collapse).
        for i in 0..n {
            if i == 0 {
                self.slots[0].SetOffset(v3(base_x(0), geo.p, 0.0))?;
                continue;
            }
            let prev: CompositionObject = self.slots[i - 1].cast()?;
            let me: CompositionObject = self.accs[i].cast()?;
            let e = self.expr(
                &format!(
                    "Vector3(q.Offset.X + {d} + {g} + a.Gap - a.Collapse, {p}, 0)",
                    d = base_x(i) - base_x(i - 1),
                    g = grow_of("q", i - 1),
                    p = geo.p
                ),
                &[("q", &prev), ("a", &me)],
            )?;
            self.slots[i].StartAnimation(&hs("Offset"), &e)?;
        }
        // Shelf shift, so the point under the pointer stays put: a running total of the
        // growth of every icon (or the part of it) that lies left of the pointer.
        for i in 0..n {
            let term = if geo.kk[i] > 0.0 {
                format!(
                    "{g} * Clamp((ptr.PointerX - {l}) / {w}, 0, 1)",
                    g = grow_of("s", i),
                    l = geo.x0 + geo.p + geo.left[i],
                    w = geo.width[i]
                )
            } else {
                "0".to_string()
            };
            let slot_obj: CompositionObject = self.slots[i].cast()?;
            let e = if i == 0 {
                self.expr(&term, &[("s", &slot_obj)])?
            } else {
                let prev: CompositionObject = self.accs[i - 1].cast()?;
                self.expr(&format!("a.Acc + {term}"), &[("s", &slot_obj), ("a", &prev)])?
            };
            self.accs[i].StartAnimation(&hs("Acc"), &e)?;
        }
        let _ = self.ptr.StopAnimation(&hs("Delta"));
        self.ptr.InsertScalar(&hs("Delta"), 0.0)?;
        let total: CompositionObject = self.accs[n - 1].cast()?;
        self.dock.StartAnimation(
            &hs("Offset"),
            // ptr.Delta: net width change while dragging; recentre by half of it.
            &self.expr(&format!("Vector3({x0} - z.Acc - ptr.Delta / 2, {y0}, 0)", x0 = geo.x0, y0 = geo.y0), &[("z", &total)])?,
        )?;
        // Shelf width: rest width + how far the last slot moved + its own growth.
        let last: CompositionObject = self.slots[n - 1].cast()?;
        let size_expr = format!(
            "Vector2({w} + (last.Offset.X - {bl}) + {g}, {h})",
            w = geo.shelf_w,
            bl = base_x(n - 1),
            g = grow_of("last", n - 1),
            h = geo.shelf_h
        );
        let size_refs = [("last", &last)];

        let (glass, geom) = self.glass(self.cfg.glass_tint, geo.r)?;
        glass.StartAnimation(&hs("Size"), &self.expr(&size_expr, &size_refs)?)?;
        geom.StartAnimation(&hs("Size"), &self.expr(&size_expr, &size_refs)?)?;
        self.dock.Children()?.InsertAtBottom(&glass)?;
        // Rim highlight as a nine-grid so the rounded ends never stretch.
        let rim_w = (geo.r * 2.0 + 4.0).ceil();
        let rim_surface = self.gfx.rim_surface(rim_w, geo.shelf_h, geo.r, self.cfg.edge_highlight)?;
        let rim_brush = c.CreateSurfaceBrushWithSurface(&rim_surface)?;
        // Without an explicit Fill the nine-grid draws the source once, unstretched, in the centre.
        rim_brush.SetStretch(CompositionStretch::Fill)?;
        let nine = c.CreateNineGridBrush()?;
        nine.SetSource(&rim_brush)?;
        nine.SetLeftInset(geo.r + 1.0)?;
        nine.SetRightInset(geo.r + 1.0)?;
        nine.SetTopInset(0.0)?;
        nine.SetBottomInset(0.0)?;
        let rim = c.CreateSpriteVisual()?;
        rim.SetBrush(&nine)?;
        rim.StartAnimation(&hs("Size"), &self.expr(&size_expr, &size_refs)?)?;
        self.dock.Children()?.InsertAtTop(&rim)?;

        for slot in &self.slots {
            self.dock.Children()?.InsertAtTop(slot)?;
        }

        // Running indicators: a small light pill under each running app, centred on the
        // (magnified) icon but not scaled with it.
        let ind_brush = c.CreateColorBrushWithColor(if crate::ui::theme::light() { Color { A: 230, R: 28, G: 28, B: 34 } } else { Color { A: 255, R: 245, G: 245, B: 247 } })?;
        let d = (4.0 * self.scale).round();
        for (i, it) in self.items.iter().enumerate() {
            if it.kind != Kind::App {
                self.inds.push(None);
                self.ind_state.push(IndState::default());
                self.atts.push(None);
                continue;
            }
            let ind = c.CreateSpriteVisual()?;
            ind.SetBrush(&ind_brush)?;
            ind.SetSize(v2(d, d))?;
            ind.SetOpacity(0.0)?;
            let g2 = c.CreateRoundedRectangleGeometry()?;
            g2.SetCornerRadius(v2(d / 2.0, d / 2.0))?;
            let ind_obj: CompositionObject = ind.cast()?;
            g2.StartAnimation(&hs("Size"), &self.expr("me.Size", &[("me", &ind_obj)])?)?;
            ind.SetClip(&c.CreateGeometricClipWithGeometry(&g2)?)?;
            let slot_obj: CompositionObject = self.slots[i].cast()?;
            let props = c.CreatePropertySet()?;
            props.InsertScalar(&hs("Shift"), 0.0)?;
            let props_obj: CompositionObject = props.cast()?;
            let dot_bottom = geo.shelf_h - (2.5 * self.scale).round();
            ind.StartAnimation(
                &hs("Offset"),
                &self.expr(
                    &format!(
                        "Vector3(s.Offset.X + {b} * s.Scale.X / 2 - me.Size.X / 2 - p.Shift, {y} - me.Size.Y, 0)",
                        b = geo.b,
                        y = dot_bottom
                    ),
                    &[("s", &slot_obj), ("me", &ind_obj), ("p", &props_obj)],
                )?,
            )?;
            let att = self.make_att(&slot_obj, props, dot_bottom)?;
            self.dock.Children()?.InsertAtTop(&att.vis)?;
            self.dock.Children()?.InsertAtTop(&ind)?;
            self.inds.push(Some(ind));
            self.ind_state.push(IndState::default());
            self.atts.push(Some(att));
        }

        self.start_panes = start_panes;
        self.start_open = false;
        for (idx, def, tile, ww) in widget_tiles {
            let h = self.geo.b;
            // Painted at its magnified size so it stays sharp when it grows.
            let res = 1.0 + self.geo.kk[idx];
            match self.gfx.surface(ww * res, h * res) {
                Ok(surface) => {
                    let sp = crate::ui::stretch_sprite(&c, &surface, ww, h)?;
                    tile.Children()?.InsertAtTop(&sp)?;
                    self.wviews.insert(idx, WView { def, surface, tile, painted: Default::default(), w: ww, h, res });
                }
                Err(e) => trace!("widget surface failed: {e:?}"),
            }
        }
        self.paint_dock_widgets(true);
        self.apply_region();
        if tracing() {
            let top = self.win_top();
            for (i, it) in self.items.iter().enumerate() {
                trace!("item {i} at ({}, {}) {:?} {}", self.monitor.left + self.geo.center_abs(i) as i32, top + (self.geo.y0 + self.geo.shelf_h / 2.0) as i32, it.kind, it.key);
            }
        }
        Ok(())
    }

    /// Hovering the Start button: a soft reflection sweeps once across the mark, top-left
    /// to bottom-right (like light passing over glass). Nothing moves if Windows'
    /// animations are off.
    fn start_hover(&mut self, on: bool) {
        if on == self.start_open || self.start_panes.is_empty() {
            return;
        }
        self.start_open = on;
        if !on || !animations_enabled() {
            return;
        }
        let c = &self.compositor;
        let Some(p) = self.start_panes.first() else { return };
        let Ok(a) = c.CreateScalarKeyFrameAnimation() else { return };
        let Ok(e) = c.CreateCubicBezierEasingFunction(v2(0.45, 0.0), v2(0.25, 1.0)) else { return };
        // The band's centre line runs where x + y = 2 * Sheen; start it before the mark's
        // top-left corner and end it past the bottom-right.
        let _ = a.InsertKeyFrame(0.0, -0.35 * p.b);
        let _ = a.InsertKeyFrameWithEasingFunction(1.0, 1.35 * p.b, &e);
        let _ = a.SetDuration(ms((850.0 * slowmo()) as i64));
        if let Ok(o) = p.fx.cast::<CompositionObject>() {
            let _ = o.StartAnimation(&hs("Sheen"), &a);
        }
    }

    /// Size of the attention dot's parts: (dot, gap to the running dot, glow box).
    fn att_metrics(&self) -> (f32, f32, f32) {
        ((4.0 * self.scale).round(), (3.0 * self.scale).round(), (22.0 * self.scale).round())
    }

    /// Build an (invisible) attention dot for one app slot. It sits just right of the
    /// running dot; when shown, the running dot slides left by `Shift` so the pair is
    /// centred under the icon.
    fn make_att(&self, slot_obj: &CompositionObject, props: CompositionPropertySet, dot_bottom: f32) -> Result<Att> {
        let c = &self.compositor;
        let (d, gap, big) = self.att_metrics();
        let green = |a: u8| Color { A: a, R: 52, G: 211, B: 94 };
        let vis = c.CreateContainerVisual()?;
        vis.SetSize(v2(big, big))?;
        vis.SetOpacity(0.0)?;

        let glow = c.CreateSpriteVisual()?;
        glow.SetSize(v2(big, big))?;
        glow.SetCenterPoint(Vector3 { X: big / 2.0, Y: big / 2.0, Z: 0.0 })?;
        let rg = c.CreateRadialGradientBrush()?;
        let stops = rg.ColorStops()?;
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(0.0, green(215))?)?;
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(0.28, green(120))?)?;
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(0.6, green(34))?)?;
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(1.0, green(0))?)?;
        glow.SetBrush(&rg)?;

        let core = c.CreateSpriteVisual()?;
        core.SetSize(v2(d, d))?;
        core.SetOffset(Vector3 { X: (big - d) / 2.0, Y: (big - d) / 2.0, Z: 0.0 })?;
        core.SetBrush(&c.CreateColorBrushWithColor(green(255))?)?;
        let g = c.CreateRoundedRectangleGeometry()?;
        g.SetSize(v2(d, d))?;
        g.SetCornerRadius(v2(d / 2.0, d / 2.0))?;
        core.SetClip(&c.CreateGeometricClipWithGeometry(&g)?)?;

        vis.Children()?.InsertAtTop(&glow)?;
        vis.Children()?.InsertAtTop(&core)?;
        // Centre of the green dot: half a dot plus half the gap right of the icon's centre
        // (the running dot's centre moves the same distance left, see `show_att`).
        vis.StartAnimation(
            &hs("Offset"),
            &self.expr(
                &format!(
                    "Vector3(s.Offset.X + {b} * s.Scale.X / 2 + {dx} - {h}, {cy} - {h}, 0)",
                    b = self.geo.b,
                    dx = (d + gap) / 2.0,
                    h = big / 2.0,
                    cy = dot_bottom - d / 2.0
                ),
                &[("s", slot_obj)],
            )?,
        )?;
        Ok(Att { vis, glow, core, props })
    }

    /// Fade an attention dot in (and start it breathing) or out.
    fn show_att(&self, att: &Att, on: bool) {
        let c = &self.compositor;
        let (d, gap, _) = self.att_metrics();
        if let Ok(p) = att.props.cast::<CompositionObject>() {
            let _ = crate::ui::animate_scalar(c, &p, "Shift", if on { (gap + d) / 2.0 } else { 0.0 }, 380);
        }
        if let Ok(v) = att.vis.cast::<CompositionObject>() {
            let _ = crate::ui::animate_scalar(c, &v, "Opacity", if on { 1.0 } else { 0.0 }, if on { 480 } else { 260 });
        }
        if on {
            let _ = self.breathe(att);
        } else {
            // The container is fading out; the glow can stop where it is.
            let _ = att.glow.StopAnimation(&hs("Opacity"));
            let _ = att.glow.StopAnimation(&hs("Scale"));
            let _ = att.core.StopAnimation(&hs("Opacity"));
        }
    }

    /// A slow, eased pulse: the glow swells and brightens, then settles, forever.
    /// With Windows' "Animation effects" turned off the glow just stays lit.
    fn breathe(&self, att: &Att) -> Result<()> {
        let c = &self.compositor;
        if !animations_enabled() {
            att.glow.SetOpacity(0.85)?;
            att.glow.SetScale(Vector3 { X: 1.0, Y: 1.0, Z: 1.0 })?;
            return Ok(());
        }
        let ease = c.CreateCubicBezierEasingFunction(v2(0.45, 0.0), v2(0.55, 1.0))?;
        let period = ms(2400);
        let op = c.CreateScalarKeyFrameAnimation()?;
        op.InsertKeyFrameWithEasingFunction(0.0, 0.3, &ease)?;
        op.InsertKeyFrameWithEasingFunction(0.5, 1.0, &ease)?;
        op.InsertKeyFrameWithEasingFunction(1.0, 0.3, &ease)?;
        op.SetDuration(period)?;
        op.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
        att.glow.StartAnimation(&hs("Opacity"), &op)?;
        let sc = c.CreateVector3KeyFrameAnimation()?;
        let s3 = |k: f32| Vector3 { X: k, Y: k, Z: 1.0 };
        sc.InsertKeyFrameWithEasingFunction(0.0, s3(0.7), &ease)?;
        sc.InsertKeyFrameWithEasingFunction(0.5, s3(1.15), &ease)?;
        sc.InsertKeyFrameWithEasingFunction(1.0, s3(0.7), &ease)?;
        sc.SetDuration(period)?;
        sc.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
        att.glow.StartAnimation(&hs("Scale"), &sc)?;
        let co = c.CreateScalarKeyFrameAnimation()?;
        co.InsertKeyFrameWithEasingFunction(0.0, 0.75, &ease)?;
        co.InsertKeyFrameWithEasingFunction(0.5, 1.0, &ease)?;
        co.InsertKeyFrameWithEasingFunction(1.0, 0.75, &ease)?;
        co.SetDuration(period)?;
        co.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
        att.core.StartAnimation(&hs("Opacity"), &co)
    }

    /// A window flashed its taskbar button (FlashWindow / FlashWindowEx), the way chat
    /// apps signal a new message. Mark it until it's focused or closed, and if the dock
    /// is auto-hidden, peek it up for a moment so the dot can be seen.
    fn on_flash(&mut self, h: HWND) {
        let representative = tracker::taskbar_ancestor(h);
        let target = self.windows.iter().find(|w| w.hwnd == h || w.hwnd == representative).map(|w| w.hwnd).unwrap_or(representative);
        // Hidden windows can't be looked at, so they never need attention. (WSL's hidden
        // "RemoteApp" window, msrdc.exe, "flashes" every few seconds; it used to keep
        // popping the auto-hidden dock up.)
        if target.is_invalid() || target == self.fg || unsafe { !IsWindowVisible(target).as_bool() } {
            return;
        }
        if !self.attention.insert(target.0 as isize) {
            return; // already marked (flashing windows send this repeatedly)
        }
        let tracked = self.windows.iter().any(|w| w.hwnd == target);
        trace!("attention: window {:?} flashed ({})", target.0, self.windows.iter().find(|w| w.hwnd == target).map(|w| w.app.as_str()).unwrap_or("not tracked yet"));
        self.update_indicators();
        // Only peek for a window the dock shows a dot for. Untracked windows (hidden helper
        // windows, tool windows) can flash over and over; they must not keep the dock up.
        if tracked && self.cfg.auto_hide && self.tucked && !self.launchpad.open && quiet_ok() {
            self.set_tucked(false);
            unsafe { SetTimer(Some(self.hwnd), T_HIDE, self.cfg.hide_delay_ms + 2600, None) };
        }
    }

    /// Animate indicator dots to match which apps are running / focused.
    fn update_indicators(&mut self) {
        let fg = self.fg;
        let items = self.items.clone();
        for (i, it) in items.iter().enumerate() {
            if i >= self.inds.len() {
                break;
            }
            let Some(ind) = self.inds[i].clone() else { continue };
            let wins = self.item_windows(it);
            let focused = wins.iter().any(|w| w.hwnd == fg);
            let attention = !focused && wins.iter().any(|w| self.attention.contains(&(w.hwnd.0 as isize)));
            let st = IndState { running: !wins.is_empty(), focused, attention };
            let prev = self.ind_state[i];
            if st == prev {
                continue;
            }
            self.ind_state[i] = st;
            if st.attention != prev.attention {
                trace!("attention dot {} for item {i} {}", if st.attention { "on" } else { "off" }, it.key);
                if let Some(Some(att)) = self.atts.get(i).cloned() {
                    self.show_att(&att, st.attention);
                }
            }
            let d = (4.0 * self.scale).round();
            let (w, op) = if st.focused { ((14.0 * self.scale).round(), 1.0) } else if st.running { (d, 0.55) } else { (d, 0.0) };
            let c = &self.compositor;
            if let (Ok(sa), Ok(oa), Ok(ease)) = (
                c.CreateVector2KeyFrameAnimation(),
                c.CreateScalarKeyFrameAnimation(),
                c.CreateCubicBezierEasingFunction(v2(0.2, 0.9), v2(0.25, 1.0)),
            ) {
                let _ = sa.InsertKeyFrameWithEasingFunction(1.0, v2(w, d), &ease);
                let _ = sa.SetDuration(ms(320));
                let _ = oa.InsertKeyFrame(1.0, op);
                let _ = oa.SetDuration(ms(220));
                let _ = ind.StartAnimation(&hs("Size"), &sa);
                let _ = ind.StartAnimation(&hs("Opacity"), &oa);
            }
        }
    }

    // ---------------------------------------------------------------- windows

    fn rescan(&mut self) {
        let t = Instant::now();
        self.windows = self.tracker.scan();
        if self.cfg.per_display_windows {
            let mine = self.monitor;
            self.windows.retain(|w| unsafe {
                let m = windows::Win32::Graphics::Gdi::MonitorFromWindow(w.hwnd, windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                let _ = GetMonitorInfoW(m, &mut mi);
                mi.rcMonitor == mine
            });
        }
        self.last_scan_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.fg = tracker::foreground();
        let fg = self.fg.0 as isize;
        let live: std::collections::HashSet<isize> = self.windows.iter().map(|w| w.hwnd.0 as isize).collect();
        self.attention.retain(|h| *h != fg && live.contains(h));
        let present: Vec<String> = self.windows.iter().map(|w| w.app.clone()).collect();
        self.running_order.retain(|a| present.contains(a));
        for a in present {
            if !self.running_order.contains(&a) {
                self.running_order.push(a);
            }
        }
        let t = Instant::now();
        self.refresh(false);
        self.last_refresh_ms = t.elapsed().as_secs_f64() * 1000.0;
    }

    // ---------------------------------------------------------------- hover, region, auto-hide

    fn region_rect(&self, hover: bool) -> RECT {
        let g = &self.geo;
        let reveal = (self.cfg.reveal_zone * self.scale).round() as i32;
        if self.tucked {
            // Tucked away, only the strip along the screen edge still takes the pointer.
            return if self.panel_top {
                RECT { left: 0, top: 0, right: g.win_w as i32, bottom: reveal }
            } else {
                RECT { left: 0, top: (g.win_h as i32 - reveal).max(0), right: g.win_w as i32, bottom: g.win_h as i32 }
            };
        }
        let (shelf_top, shelf_bottom) = (g.y0 as i32, (g.y0 + g.shelf_h) as i32);
        let win_h = g.win_h as i32;
        // The part of the window the icons never reach is the magnification headroom; it
        // lies above the shelf at the bottom of the screen and below it at the top. While
        // auto-hiding, the margin towards the screen edge counts too, so the pointer
        // resting on that edge still counts as "on the dock".
        let (top, bottom) = if self.panel_top {
            (if self.cfg.auto_hide { 0 } else { shelf_top }, if hover { win_h } else { shelf_bottom })
        } else {
            (if hover { 0 } else { shelf_top }, if self.cfg.auto_hide { win_h } else { shelf_bottom })
        };
        let (left, right) = if hover {
            (g.hover_lr.0, g.hover_lr.1)
        } else {
            (g.x0 as i32, (g.x0 + g.shelf_w) as i32)
        };
        RECT { left, top, right, bottom }
    }

    fn apply_region(&self) {
        let r = self.region_rect(self.hovering);
        trace!("region {}x{} at ({}, {}) tucked={} hovering={}", r.right - r.left, r.bottom - r.top, r.left, r.top, self.tucked, self.hovering);
        unsafe {
            let rgn = CreateRectRgn(r.left, r.top, r.right, r.bottom);
            SetWindowRgn(self.hwnd, Some(rgn), true);
        }
    }

    fn animate_amount(&self, to: f32, duration_ms: i64) -> Result<()> {
        if testctl() {
            // Test captures (PrintWindow on a locked session) don't advance keyframes.
            let _ = self.ptr.StopAnimation(&hs("Amount"));
            return self.ptr.InsertScalar(&hs("Amount"), to);
        }
        let c = &self.compositor;
        let a = c.CreateScalarKeyFrameAnimation()?;
        let ease = c.CreateCubicBezierEasingFunction(v2(0.25, 0.8), v2(0.25, 1.0))?;
        a.InsertKeyFrameWithEasingFunction(1.0, to, &ease)?;
        a.SetDuration(ms(duration_ms))?;
        self.ptr.StartAnimation(&hs("Amount"), &a)
    }

    /// Slide the whole dock off the bottom of the screen (or back) on a spring.
    fn set_tucked(&mut self, tucked: bool) {
        trace!("set_tucked({tucked}) was {}", self.tucked);
        if self.tucked == tucked {
            return;
        }
        self.tucked = tucked;
        let c = &self.compositor;
        // Slide only as far as needed to clear the shelf (the window also holds empty
        // headroom for magnified icons; sliding that too made the visible part vanish
        // in the first ~100 ms). Each keyframe is relative to the starting value, so an
        // interrupted hide/show continues smoothly from wherever the dock is.
        let distance = (self.geo.shelf_h + self.geo.margin + 10.0 * self.scale).ceil()
            * if self.panel_top { -1.0 } else { 1.0 };
        let to = if tucked { distance } else { 0.0 };
        if let (Ok(a), Ok(lin)) = (c.CreateVector3KeyFrameAnimation(), c.CreateLinearEasingFunction()) {
            let n = 48;
            for k in 1..=n {
                let t = k as f32 / n as f32;
                // Hide: smooth ease-in-out. Show: a spring with a little overshoot.
                let p = if tucked { ease_in_out(t) } else { spring(t, 0.16) };
                let e = format!("this.StartingValue + (Vector3(0, {to}, 0) - this.StartingValue) * {p}");
                let _ = a.InsertExpressionKeyFrameWithEasingFunction(t, &hs(&e), &lin);
            }
            let dur = if tucked { TUCK_MS } else { SHOW_MS };
            let _ = a.SetDuration(ms((dur as f32 * slowmo()) as i64));
            let _ = self.root.StartAnimation(&hs("Offset"), &a);
        }
        if tucked {
            self.hovering = false;
            self.hover = None;
            let _ = self.show_label(None);
            let _ = self.animate_amount(0.0, 200);
            // The window region also clips drawing: shrinking it now would cut the slide-out
            // off after a frame or two. Shrink it once the dock is fully off-screen.
            unsafe { SetTimer(Some(self.hwnd), T_TUCKED, (TUCK_MS as f32 * slowmo()) as u32 + 30, None) };
        } else {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), T_TUCKED);
            }
            self.apply_region();
        }
    }

    fn on_move(&mut self, x: f32, y: f32) {
        if !self.hovering {
            trace!("pointer enters at ({x}, {y}) tucked={}", self.tucked);
        }
        if self.tucked {
            // Reveal after the pointer rests in the bottom band for reveal_delay.
            unsafe { SetTimer(Some(self.hwnd), T_REVEAL, self.cfg.reveal_delay_ms.max(1), None) };
            return;
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_HIDE);
        }
        if !self.hovering {
            self.hovering = true;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), T_SHRINK);
            }
            // Simulated pointers (--measure, --demo-hover, LAMINAR_TESTCTL) must not register
            // for real leave events.
            if !self.synthetic() && !testctl() {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: self.hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut tme);
                }
            }
            self.apply_region();
            let _ = self.animate_amount(1.0, 160);
        }
        // Which item is under the pointer, measured before any magnification: the answer
        // doesn't change as the icons grow, so neither does the layout.
        let hit = self.geo.hit_base(x, y);
        let tray_key = hit.and_then(|i| match self.items.get(i).map(|item| item.kind) {
            Some(Kind::Tray(key)) => Some(key),
            _ => None,
        });
        self.set_tray_hover(tray_key, false);
        if self.tray_popup.open && !hit.map(|i| self.items[i].kind == Kind::TrayOverflow).unwrap_or(false) {
            self.hide_tray_popup();
        }
        // The peak follows the pointer continuously: the compositor then interpolates every
        // frame and the icons glide. Snapping it to slot centres (and writing it only when
        // the slot changed) made the whole bar step at a few frames a second, which is worse
        // than the twitch it was meant to cure — that twitch came from *hit testing* the
        // magnified layout, and that is what `hit_base` above fixes.
        self.anchor_x = x;
        let _ = self.ptr.InsertScalar(&hs("PointerX"), x);
        self.ptr_xy = (x, y);
        if hit != self.hover {
            let launch = |h: Option<usize>| h.map(|i| self.items.get(i).map(|it| it.kind == Kind::Launch).unwrap_or(false)).unwrap_or(false);
            self.start_hover(launch(hit));
            self.hover = hit;
            self.preview_hover(hit);
            let standard_tip = hit.map(|i| match self.items[i].kind {
                Kind::Tray(key) => self.tray_items.iter().find(|icon| icon.key == key).map(|icon| icon.standard_tip && !icon.tip.is_empty()).unwrap_or(false),
                _ => true,
            }).unwrap_or(false);
            let label_hit = if self.preview.open || !standard_tip { None } else { hit };
            let _ = self.show_label(label_hit);
        }
    }

    /// While hovering, the dock's window covers the headroom above the shelf (so magnified
    /// icons stay under the pointer). With Launchpad open that headroom lies over Launchpad's
    /// bottom bar (account, Settings, Power): a point there that isn't on a magnified item is
    /// see-through, so it reaches Launchpad (same thread, so HTTRANSPARENT passes it on).
    fn headroom_passes_through(&self, x: f32, y: f32) -> bool {
        // Not gated on `hovering`: handing the pointer to Launchpad makes Windows report a
        // leave, and the next move must not bounce back into the dock (hover flicker).
        if !self.launchpad.open || self.drag.is_some() || self.tucked {
            return false;
        }
        let g = &self.geo;
        // Inside the shelf's own strip the panel keeps the pointer.
        if g.top {
            if y <= g.y0 + g.shelf_h {
                return false;
            }
        } else if y >= g.y0 {
            return false;
        }
        // The far edge of whatever is drawn at x (magnified item, or the shelf).
        let (dx, scales) = g.layout(if self.hovering { self.anchor_x } else { x }, if self.hovering { 1.0 } else { 0.0 });
        let mut acc = 0.0;
        let mut top = g.y0;
        let mut bottom = g.y0 + g.shelf_h;
        for i in 0..g.left.len() {
            let l = dx + g.p + g.left[i] + acc;
            let slot = g.width[i] + (scales[i] - 1.0) * g.vw[i];
            if x >= l && x < l + slot && (g.icon[i] || g.wid[i]) {
                let (t, b) = g.item_box(scales[i]);
                top = top.min(t);
                bottom = bottom.max(b);
            }
            acc += (scales[i] - 1.0) * g.vw[i];
        }
        // Same margin past an item as `Geo::hit` gives it.
        if g.top {
            y > bottom + g.g
        } else {
            y < top - g.g
        }
    }

    fn on_leave(&mut self) {
        trace!("pointer leaves");
        if self.menu_open || self.drag.is_some() || self.tray_drag.is_some() {
            return;
        }
        let mut point = POINT::default();
        let _ = get_cursor_pos(&mut point);
        let popup = self.tray_popup.bounds();
        let inside_tray_popup = (self.tray_popup.open
            && point.x >= popup.left && point.x < popup.right
            && point.y >= popup.top && point.y < popup.bottom)
            || self.mini_tray.contains(point.x, point.y);
        if !inside_tray_popup {
            self.set_tray_hover(None, false);
            if self.tray_popup.open { self.hide_tray_popup(); }
        }
        self.hovering = false;
        self.hover = None;
        self.pressed = None;
        self.start_hover(false);
        let _ = self.show_label(None);
        let _ = self.animate_amount(0.0, 260);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_PREVIEW);
            if self.preview.open {
                SetTimer(Some(self.hwnd), T_PREVIEW_HIDE, 300, None);
            }
        }
        unsafe {
            SetTimer(Some(self.hwnd), T_SHRINK, 320, None);
            if self.cfg.auto_hide {
                SetTimer(Some(self.hwnd), T_HIDE, self.cfg.hide_delay_ms.max(1), None);
            }
        }
    }

    /// Start hover tracking afresh from where the pointer really is: over the dock, it
    /// hovers (and Windows will report when it leaves); elsewhere, it leaves (auto-hide).
    fn resync_hover(&mut self) {
        let mut p = POINT::default();
        let _ = get_cursor_pos(&mut p);
        let win_top = self.win_top();
        let (x, y) = (p.x - self.monitor.left, p.y - win_top);
        let r = self.region_rect(true);
        let inside = !self.tucked
            && x >= r.left
            && x < r.right
            && y >= r.top
            && y < r.bottom
            && unsafe { WindowFromPoint(p) } == self.hwnd;
        trace!("resync hover: pointer at ({x}, {y}) inside={inside}");
        self.hovering = false;
        if inside {
            self.on_move(x as f32, y as f32);
        } else {
            self.on_leave();
        }
    }

    fn on_reveal_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_REVEAL);
            let mut p = POINT::default();
            let _ = get_cursor_pos(&mut p);
            let zone = (self.cfg.reveal_zone * self.scale).round() as i32 + 1;
            let at_edge = if self.panel_top { p.y <= self.floor_y + zone } else { p.y >= self.floor_y - zone };
            if at_edge && p.x >= self.monitor.left && p.x < self.monitor.right {
                trace!("reveal: pointer rested at the screen edge ({}, {})", p.x, p.y);
                self.set_tucked(false);
                // If the pointer never reaches the shelf, hide again.
                SetTimer(Some(self.hwnd), T_HIDE, self.cfg.hide_delay_ms + 900, None);
            }
        }
    }

    fn show_label(&mut self, idx: Option<usize>) -> Result<()> {
        let idx = idx.filter(|&i| i < self.items.len() && i < self.slots.len());
        let c = &self.compositor;
        let fade = c.CreateScalarKeyFrameAnimation()?;
        fade.SetDuration(ms(120))?;
        let Some(i) = idx else {
            fade.InsertKeyFrame(1.0, 0.0)?;
            return self.label.StartAnimation(&hs("Opacity"), &fade);
        };
        let s = self.scale;
        let it = &self.items[i];
        let name = match it.kind {
            Kind::Launch => "Launchpad".to_string(),
            Kind::Stack => self.stack_of(&it.key).map(|v| v.1).unwrap_or_else(|| "Stack".into()),
            Kind::Widget => widget_of(&it.key).map(|d| d.name.to_string()).unwrap_or_else(|| "Widget".into()),
            Kind::Tray(key) => self.tray_items.iter().find(|icon| icon.key == key)
                .map(|icon| icon.tip.lines().next().unwrap_or_default().chars().take(70).collect::<String>())
                .filter(|tip| !tip.trim().is_empty())
                .unwrap_or_else(|| crate::strings::tr("Background app")),
            Kind::TrayOverflow => crate::strings::tr(&format!("Show hidden tray icons ({})", self.tray_overflow_keys.len())),
            _ => self.names.get(&it.key).cloned().unwrap_or_else(|| it.key.clone()),
        };
        // Centred over the item's drawn width (a widget is wider than an icon).
        let iw = self.geo.vw.get(i).copied().unwrap_or(self.geo.b).max(self.geo.b);
        let (surface, tw, th) = self.gfx.text_surface(&name, 13.0 * s)?;
        self.label_text.SetBrush(&c.CreateSurfaceBrushWithSurface(&surface)?)?;
        let (padx, pady) = (11.0 * s, 5.0 * s);
        let (lw, lh) = ((tw + 2.0 * padx).round(), (th + 2.0 * pady).round());
        self.label_text.SetSize(v2(tw, th))?;
        self.label_text.SetOffset(v3(padx, pady, 0.0))?;
        self.label.SetSize(v2(lw, lh))?;
        self.label_geom.SetSize(v2(lw, lh))?;
        let b = self.geo.b;
        // Below the icon when the shelf sits at the top, above it when it sits at the bottom.
        let ly = if self.panel_top {
            format!("d.Offset.Y + t.Offset.Y + {b} * t.Scale.Y + {gap}", gap = 10.0 * s)
        } else {
            format!("d.Offset.Y + t.Offset.Y + {b} - {b} * t.Scale.Y - {lh} - {gap}", gap = 10.0 * s)
        };
        let e = self.expr(
            &format!(
                "Vector3(d.Offset.X + t.Offset.X + {iw} * t.Scale.X / 2 - {hw}, {ly}, 0)",
                hw = lw / 2.0
            ),
            &[("d", &self.dock.cast()?), ("t", &self.slots[i].cast()?)],
        )?;
        self.label.StartAnimation(&hs("Offset"), &e)?;
        fade.InsertKeyFrame(1.0, 1.0)?;
        self.label.StartAnimation(&hs("Opacity"), &fade)
    }

    fn bounce(&self, i: usize) -> Result<()> {
        let Some(tile) = &self.tiles[i] else { return Ok(()) };
        let c = &self.compositor;
        let a = c.CreateScalarKeyFrameAnimation()?;
        let b = self.geo.b;
        let out = c.CreateCubicBezierEasingFunction(v2(0.2, 0.7), v2(0.4, 1.0))?;
        let inn = c.CreateCubicBezierEasingFunction(v2(0.6, 0.0), v2(0.8, 0.3))?;
        for (t, v, e) in [(0.22, -0.55, &out), (0.44, 0.0, &inn), (0.62, -0.28, &out), (0.8, 0.0, &inn), (1.0, 0.0, &out)] {
            a.InsertKeyFrameWithEasingFunction(t, v * b, e)?;
        }
        a.SetDuration(ms(1000))?;
        tile.StartAnimation(&hs("Offset.Y"), &a)
    }

    // ---------------------------------------------------------------- clicks

    /// Windows taskbar behaviour: launch if closed; bring to front if behind;
    /// minimize (or cycle windows) if already in front.
    fn on_click(&mut self, i: usize) {
        self.hide_preview();
        let it = self.items[i].clone();
        if it.kind != Kind::TrayOverflow {
            self.hide_tray_popup();
        }
        // Opening or switching to something from the dock leaves Launchpad. (Clicking the
        // dock doesn't take focus from Launchpad, so it wouldn't close on its own.)
        if self.launchpad.open && matches!(it.kind, Kind::App | Kind::Stack) {
            self.close_launchpad();
        }
        match it.kind {
            Kind::Launch => self.toggle_launchpad(),
            Kind::Update => self.update_menu(),
            Kind::Tray(key) => self.tray_click(key, crate::tray::Click::Left),
            Kind::TrayOverflow => {
                if self.tray_popup.open { self.hide_tray_popup(); } else { self.show_tray_popup(); }
            }
            Kind::Separator => {}
            Kind::Widget => self.widget_click(i),
            Kind::Stack => {
                if self.stack_view.open {
                    self.stack_view.hide();
                } else {
                    self.open_stack(i);
                }
            }
            Kind::App => {
                let wins = self.item_windows(&it);
                let fg = tracker::foreground();
                if wins.is_empty() {
                    let _ = self.bounce(i);
                    apps::launch(&it.key);
                } else if wins.iter().any(|w| w.hwnd == fg) {
                    if wins.len() == 1 && self.cfg.click_to_minimize {
                        tracker::minimize(wins[0].hwnd);
                    } else {
                        // Bring the back-most window forward: repeated clicks cycle through them.
                        tracker::activate(wins[wins.len() - 1].hwnd);
                    }
                } else {
                    tracker::activate(wins[0].hwnd);
                }
            }
        }
    }

    /// Right-click: build the menu for whatever is under the pointer and open it above the dock.
    fn context_menu(&mut self, x: f32, y: f32, anchor: Option<POINT>) {
        let hit = self.geo.hit(if self.hovering { self.anchor_x } else { x }, y, if self.hovering { 1.0 } else { 0.0 });
        let item = hit.map(|i| self.items[i].clone());
        match item.as_ref().map(|item| item.kind) {
            Some(Kind::Tray(key)) => {
                self.tray_click(key, crate::tray::Click::Right);
                return;
            }
            Some(Kind::TrayOverflow) => {
                if self.tray_popup.open { self.hide_tray_popup(); } else { self.show_tray_popup(); }
                return;
            }
            _ => {}
        }
        let app = item.clone().filter(|it| it.kind == Kind::App);
        let wins = app.as_ref().map(|it| self.item_windows(it)).unwrap_or_default();
        let mut m: Vec<MenuItem> = Vec::new();
        if app.is_none() {
            m.push(MenuItem::item("Control Center", CMD_CONTROL_CENTER));
            m.push(MenuItem::sep());
        }
        if let Some(it) = &app {
            let name = self.names.get(&it.key).cloned().unwrap_or_else(|| it.key.clone());
            m.push(MenuItem::header(name));
            if !wins.is_empty() {
                for (i, w) in wins.iter().enumerate().take(8) {
                    let title = if w.title.chars().count() > 48 {
                        format!("{}…", w.title.chars().take(47).collect::<String>())
                    } else {
                        w.title.clone()
                    };
                    m.push(MenuItem::item(title, CMD_WINDOW_BASE + i).checked(w.hwnd == self.fg));
                }
                m.push(MenuItem::sep());
            }
            m.push(MenuItem::item("Open New Window", CMD_NEW).shortcut("Shift+Click"));
            m.push(MenuItem::sep());
            m.push(MenuItem::item("Keep in Dock", CMD_PIN).checked(it.pinned));
            if it.pinned {
                m.push(MenuItem::item("Add Separator After", CMD_ADD_SEP));
                for (n, p) in self.cfg.pins.iter().enumerate() {
                    if let Pin::Stack { stack, .. } = p {
                        m.push(MenuItem::item(format!("Move to \u{201c}{stack}\u{201d}"), CMD_ADD_TO_STACK_BASE + n));
                    }
                }
                m.push(MenuItem::item("New Stack", CMD_NEW_STACK));
            }
            if !wins.is_empty() {
                m.push(MenuItem::sep());
                let label = if wins.len() == 1 { "Close Window".to_string() } else { format!("Close All {} Windows", wins.len()) };
                m.push(MenuItem::item(label, CMD_CLOSE).danger());
            }
            m.push(MenuItem::sep());
        } else if let Some((n, name, _)) = item.as_ref().filter(|it| it.kind == Kind::Stack).and_then(|it| self.stack_of(&it.key)) {
            m.push(MenuItem::header(name));
            m.push(MenuItem::item("Open", CMD_STACK_OPEN));
            m.push(MenuItem::item("Rename Stack…", CMD_STACK_RENAME));
            m.push(MenuItem::item("Ungroup Stack", CMD_STACK_UNGROUP));
            m.push(MenuItem::item("Remove from Dock", CMD_STACK_REMOVE).danger());
            m.push(MenuItem::sep());
            self.menu_stack = Some(n);
        } else if let Some((it, def)) = item.as_ref().filter(|it| it.kind == Kind::Widget).and_then(|it| widget_of(&it.key).map(|d| (it.clone(), d))) {
            m.push(MenuItem::header(def.name.to_string()));
            self.menu_widget = widget_pin(&it.key);
            if crate::widgets::conns(def).iter().any(|c| laminar_sys::connectors::info(c).map(|i| !i.fields.is_empty()).unwrap_or(false)) {
                m.push(MenuItem::item("Connect…", CMD_WIDGET_CONNECT));
            }
            if let Some((_, label)) = crate::widgets::local_form(def) {
                m.push(MenuItem::item(label, dock_widgets::CMD_WIDGET_LOCAL));
            }
            if matches!(def.face, crate::widgets::catalog::Face::Timer(_)) {
                m.push(MenuItem::item("Reset", CMD_WIDGET_RESET));
            }
            if matches!(def.face, crate::widgets::catalog::Face::Timer(crate::widgets::TimerKind::Countdown)) {
                let cur = self.hub.timers.get(&crate::widgets::TimerKind::Countdown).map(|t| t.total as usize / 60).unwrap_or(0);
                for mins in TIMER_MINUTES {
                    m.push(MenuItem::item(format!("{mins} min"), CMD_TIMER_MIN_BASE + mins).checked(mins == cur));
                }
            }
            m.push(MenuItem::sep());
            m.extend(self.dock_size_items(&it.key));
            m.push(MenuItem::sep());
            m.push(MenuItem::item("Remove from Dock", CMD_WIDGET_REMOVE).danger());
            m.push(MenuItem::sep());
        } else if item.as_ref().map(|it| it.kind == Kind::Update).unwrap_or(false) {
            m.extend(self.update_menu_items());
            m.push(MenuItem::sep());
        } else if item.as_ref().map(|it| it.kind == Kind::Launch).unwrap_or(false) {
            m.push(MenuItem::item("Task Manager", CMD_TASKMGR).shortcut("Ctrl+Shift+Esc"));
            m.push(MenuItem::item("Settings", CMD_WINSETTINGS));
            m.push(MenuItem::item("File Explorer", CMD_EXPLORER));
            m.push(MenuItem::item("Terminal", CMD_TERMINAL));
            m.push(MenuItem::sep());
        }
        if let Some(sep) = self.separator_at(x) {
            m.push(MenuItem::item("Remove Separator", CMD_REMOVE_SEP_BASE + sep));
            m.push(MenuItem::sep());
        }
        m.push(MenuItem::item("Automatically Hide the Dock", CMD_AUTOHIDE).checked(self.cfg.auto_hide));
        m.push(MenuItem::item("Magnification", CMD_MAGNIFY).checked(self.cfg.magnify));
        m.push(MenuItem::item("Hide the Windows Taskbar", CMD_TASKBAR).checked(self.cfg.hide_windows_taskbar));
        m.push(MenuItem::item("Clock on the Desktop", CMD_CLOCK_DESKTOP).checked(self.cfg.desktop_clock));
        m.push(MenuItem::item("Карточка нагрузки на рабочем столе", CMD_SYSTEM_DESKTOP).checked(self.cfg.desktop_system));
        m.push(MenuItem::sep());
        // Widgets: the everyday ones are one click away, the rest live in the gallery.
        m.push(MenuItem::header("Widgets on the Panel"));
        for (index, id) in DOCK_WIDGETS.iter().enumerate() {
            let id: &str = id;
            let Some(def) = crate::widgets::catalog::get(id) else { continue };
            let present = self.cfg.pins.iter().any(|p| matches!(p, Pin::Widget { widget, .. } if widget.as_str() == id));
            m.push(MenuItem::item(def.name, CMD_WIDGET_TOGGLE_BASE + index).checked(present));
        }
        m.push(MenuItem::sep());
        m.push(MenuItem::item("Dock Settings…", CMD_SETTINGS));
        if crate::updater::installed() {
        }
        m.push(MenuItem::item("Quit AuraDock", CMD_QUIT));

        self.hide_preview();
        self.stack_view.hide();
        if !item.as_ref().map(|it| it.kind == Kind::Stack).unwrap_or(false) {
            self.menu_stack = None;
        }
        self.menu_ctx = Some((app, wins));
        self.menu_open = true;
        let mut p = POINT::default();
        let _ = get_cursor_pos(&mut p);
        let p = anchor.unwrap_or(p);
        if let Err(e) = self.menu.show(&self.compositor, &self.gfx, &self.backdrop, m, p, !self.panel_top, self.scale, self.monitor) {
            trace!("menu failed: {e:?}");
            self.menu_open = false;
        }
    }

    /// Index into cfg.pins of a separator the pointer is on, if any.
    fn separator_at(&self, x: f32) -> Option<usize> {
        let px = if self.hovering { self.anchor_x } else { x };
        let (dx, scales) = self.geo.layout(px, if self.hovering { 1.0 } else { 0.0 });
        let mut acc = 0.0;
        for (i, it) in self.items.iter().enumerate() {
            let l = dx + self.geo.p + self.geo.left[i] + acc;
            if it.kind == Kind::Separator && it.pinned && x >= l - 4.0 && x < l + self.geo.sep_w + 4.0 {
                return it.key.strip_prefix("::sep").and_then(|n| n.parse().ok());
            }
            acc += (scales[i] - 1.0) * self.geo.b;
        }
        None
    }

    /// Test modes drive a simulated pointer; real pointer-leave events must be ignored.
    fn synthetic(&self) -> bool {
        self.opts.measure || self.opts.demo_hover || self.opts.demo.is_some()
    }

    /// --demo NAME: open a surface automatically so it can be captured and checked.
    fn run_demo(&mut self, name: &str) {
        // Test docks only: LAMINAR_DEMO_UPDATE=0.1.1 shows the update icon as if that version were ready.
        if let Ok(v) = std::env::var("LAMINAR_DEMO_UPDATE") {
            self.app_update = Some(v);
            self.refresh(true);
        }
        let g = &self.geo;
        let i = 4.min(g.left.len().saturating_sub(1));
        let (x, y) = (g.center_abs(i), g.y0 + g.shelf_h / 2.0);
        let anchor = POINT { x: self.monitor.left + x as i32, y: self.shelf_inner_y() };
        match name {
            "menu" => {
                self.on_move(x, y);
                self.context_menu(x, y, Some(anchor));
            }
            "start-hover" => {
                let (sx, sy) = (g.center_abs(0), g.y0 + g.shelf_h / 2.0);
                self.on_move(sx, sy);
            }
            "hover-item" => {
                let k = std::env::var("LAMINAR_DEMO_ITEM").ok().and_then(|v| v.parse().ok()).unwrap_or(1usize).min(g.left.len().saturating_sub(1));
                let (sx, sy) = (g.center_abs(k), g.y0 + g.shelf_h / 2.0);
                self.on_move(sx, sy);
            }
            "lp-gallery" | "lp-form" => {
                self.open_launchpad();
                if name == "lp-form" {
                    let conn = std::env::var("LAMINAR_DEMO_CONN").unwrap_or_else(|_| "stripe".into());
                    if let Some(info) = laminar_sys::connectors::info(&conn) {
                        self.open_connect(info.id);
                    }
                } else {
                    let k = std::env::var("LAMINAR_DEMO_CAT").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
                    self.lp_act(Act::AddWidgets);
                    self.lp_act(Act::GalleryCat(k));
                }
            }
            "launchpad" | "lp-all" | "lp-search" | "lp-settings" | "lp-about" | "lp-wifi" | "lp-wifi-sample" => {
                self.open_launchpad();
                match name {
                    "lp-all" => self.lp_act(Act::ToggleAll),
                    "lp-search" => {
                        for ch in "code".chars() {
                            self.launchpad.key_char(ch);
                        }
                        self.lp_query_changed();
                    }
                    "lp-settings" => self.lp_act(Act::OpenSettings),
                    "lp-about" => {
                        self.lp_act(Act::OpenSettings);
                        self.lp_act(Act::Section(3));
                    }
                    "lp-wifi" => self.lp_act(Act::WifiMenu),
                    "lp-wifi-sample" => {
                        // Layout check with made-up networks (real replies are ignored in this mode).
                        use laminar_sys::radios::WifiNetwork;
                        let net = |ssid: &str, signal_percent, secured, connected, known| WifiNetwork { ssid: ssid.into(), signal_percent, secured, connected, known };
                        self.launchpad.wifi_open = true;
                        self.wifi = Some(Ok(vec![
                            net("Home Network", 92, true, true, true),
                            net("Home Network 5G", 80, true, false, true),
                            net("Coffee Shop Guest", 61, false, false, false),
                            net("Neighbour-2.4", 38, true, false, false),
                            net("Printer-Direct-7F", 12, true, false, false),
                        ]));
                        self.control.wifi = Some(true);
                        self.lp_render("all");
                    }
                    _ => {}
                }
            }
            "stack" => {
                if let Some(j) = self.items.iter().position(|it| it.kind == Kind::Stack) {
                    let (px, py) = (self.geo.center_abs(j), self.geo.y0 + self.geo.shelf_h / 2.0);
                    self.on_move(px, py);
                    self.open_stack(j);
                }
            }
            "preview" => {
                // The first running app with the most windows.
                let items = self.items.clone();
                let best = (0..items.len()).filter(|&j| items[j].kind == Kind::App).max_by_key(|&j| self.item_windows(&items[j]).len());
                if let Some(j) = best {
                    let (px, py) = (self.geo.center_abs(j), self.geo.y0 + self.geo.shelf_h / 2.0);
                    self.on_move(px, py);
                    self.show_preview(j);
                }
            }
            _ => {}
        }
    }

    fn menu_closed(&mut self) {
        self.menu_open = false;
        // Windows sent its one "pointer left" notice while the pointer was on the menu (and
        // it was ignored then), so start hover tracking afresh from where the pointer is.
        // Just checking "is it over the dock?" wasn't enough: a menu item sitting over the
        // dock's window left the dock thinking it was hovered, with no leave notice to come.
        if self.drag.is_none() {
            self.resync_hover();
        }
    }

    // ---------------------------------------------------------------- Laminar updates

    fn update_menu_items(&self) -> Vec<MenuItem> {
        let v = self.app_update.clone().unwrap_or_default();
        vec![
            MenuItem::header(format!("AuraDock {v} is ready to install")),
            MenuItem::item("Restart and Update", CMD_UPDATE_NOW),
            MenuItem::item("What's New", CMD_UPDATE_NOTES),
            MenuItem::sep(),
            MenuItem::item("Remind Me Tomorrow", CMD_UPDATE_TOMORROW),
            MenuItem::item("Remind Me Next Week", CMD_UPDATE_WEEK),
            MenuItem::item("Skip This Version", CMD_UPDATE_SKIP),
        ]
    }

    /// A small message menu above the dock (update results).
    fn notice(&mut self, lines: &[String]) {
        let mut m: Vec<MenuItem> = Vec::new();
        for (k, l) in lines.iter().enumerate() {
            m.push(if k == 0 { MenuItem::header(l.clone()) } else { MenuItem { enabled: false, ..MenuItem::item(l.clone(), 0) } });
        }
        m.push(MenuItem::sep());
        m.push(MenuItem::item("OK", 0));
        self.open_menu_at_cursor(m);
    }

    fn update_menu(&mut self) {
        let m = self.update_menu_items();
        self.open_menu_at_cursor(m);
    }

    fn open_menu_at_cursor(&mut self, m: Vec<MenuItem>) {
        self.hide_preview();
        self.stack_view.hide();
        self.menu_stack = None;
        self.menu_ctx = None;
        self.menu_open = true;
        let mut p = POINT::default();
        let _ = get_cursor_pos(&mut p);
        if let Err(e) = self.menu.show(&self.compositor, &self.gfx, &self.backdrop, m, p, !self.panel_top, self.scale, self.monitor) {
            trace!("menu failed: {e:?}");
            self.menu_open = false;
        }
    }

    /// Automatic update checks, at the frequency set in Settings → About (default once a
    /// day; "At startup" checks once per launch). The last check time is saved, so
    /// restarting Laminar doesn't check again. People on metered connections asked for this.
    fn auto_update_tick(&mut self) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static RAN: AtomicBool = AtomicBool::new(false);
        if !crate::updater::installed() {
            return;
        }
        let first = !RAN.swap(true, Ordering::SeqCst);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let last = self.cfg.last_update_check;
        let due = crate::updater::check_due(self.cfg.check_updates, self.cfg.update_check_hours, last, now, first);
        if due {
            self.cfg.last_update_check = now;
            if let Err(error) = self.cfg.save() { trace!("dock settings were not saved: {error}"); }
            self.check_for_update(false);
        } else if first {
            // Not checking now, but a version downloaded earlier still gets its dock icon.
            crate::updater::show_pending(self.hwnd, WM_APP_UPDATE);
        }
    }

    /// Ask GitHub for a newer Laminar (installed copies only). `manual` (the dock menu)
    /// reports the result in a menu; otherwise it only updates the icon and Settings.
    fn check_for_update(&mut self, manual: bool) {
        if !crate::updater::installed() {
            if manual {
                self.notice(&["This copy of AuraDock can't update itself".into(), "Only copies installed with Setup.exe check for updates.".into()]);
            }
            return;
        }
        self.app_update_manual |= manual;
        trace!("update check started (manual={manual})");
        crate::updater::check(self.hwnd, WM_APP_UPDATE);
        self.lp_render_about();
    }

    fn on_update_checked(&mut self) {
        use crate::updater::Status;
        let st = crate::updater::status();
        trace!("update check: {st:?}");
        let manual = self.app_update_manual;
        match &st {
            Status::Ready(v) => {
                self.app_update = Some(v.clone());
                self.refresh(false);
                if manual {
                    self.update_menu();
                }
            }
            Status::UpToDate if manual => {
                self.app_update_manual = false;
                self.notice(&[format!("AuraDock {} is up to date", crate::updater::version_label())]);
            }
            Status::Failed(e) if manual => {
                self.app_update_manual = false;
                self.notice(&["Couldn't check for updates".into(), e.chars().take(80).collect()]);
            }
            _ => self.app_update_manual = false,
        }
        self.lp_render_about();
    }

    /// Settings → About shows the update status; redraw it if it's on screen.
    fn lp_render_about(&mut self) {
        if self.launchpad.open && self.launchpad.settings == Some(3) {
            self.lp_render("all");
        }
    }

    fn update_status_line(&self) -> String {
        use crate::updater::Status;
        match crate::updater::status() {
            Status::NotInstalled => "Developer build: updates are off.".into(),
            Status::Idle => if crate::updater::installed() { "Updates: not checked yet.".into() } else { "Developer build: updates are off.".into() },
            Status::Checking => "Checking for updates…".into(),
            Status::UpToDate => "Up to date.".into(),
            Status::Ready(v) => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                if v == self.cfg.update_skip {
                    format!("Version {v} is ready to install. You skipped it, so the dock won't remind you.")
                } else if now < self.cfg.update_snooze_until {
                    let days = (self.cfg.update_snooze_until - now).div_ceil(24 * 3600);
                    format!("Version {v} is ready to install. The dock will remind you in {days} day{}.", if days == 1 { "" } else { "s" })
                } else {
                    format!("Version {v} is ready to install.")
                }
            }
            Status::Failed(e) => format!("Update check failed: {}", e.chars().take(70).collect::<String>()),
        }
    }

    /// The update controls Settings → About shows for the current state.
    fn update_ui_for(ready: &Option<String>) -> lp::UpdateUi<'_> {
        use crate::updater::Status;
        // Test driver only: LAMINAR_DEMO_UPDATE=<version>|checking|idle shows that state
        // (test copies aren't installed, so they'd otherwise show "Developer build").
        if testctl() {
            static DEMO: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
            if let Some(v) = DEMO.get_or_init(|| std::env::var("LAMINAR_DEMO_UPDATE").ok()).as_deref() {
                return match v {
                    "checking" => lp::UpdateUi::Checking,
                    "idle" => lp::UpdateUi::Idle,
                    v => lp::UpdateUi::Ready(v),
                };
            }
        }
        if !crate::updater::installed() {
            return lp::UpdateUi::Off;
        }
        match crate::updater::status() {
            Status::Checking => lp::UpdateUi::Checking,
            Status::Ready(_) => ready.as_deref().map(lp::UpdateUi::Ready).unwrap_or(lp::UpdateUi::Idle),
            _ => lp::UpdateUi::Idle,
        }
    }

    fn snooze_update(&mut self, secs: u64) {
        let until = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) + secs;
        self.app_update_manual = false;
        self.save_and_apply(|c| c.update_snooze_until = until);
        self.refresh(false);
    }

    fn save_and_apply(&mut self, f: impl FnOnce(&mut Config)) {
        let mut cfg = self.cfg.clone();
        f(&mut cfg);
        // Same rules as loading the file (e.g. always-on widgets appear or go at once).
        let cfg = cfg.sanitized();
        if let Err(error) = cfg.save() {
            report_settings_error(self.hwnd, &error);
            return;
        }
        self.apply_config(cfg);
    }

    fn on_menu_command(&mut self, cmd: usize) {
        if self.widget_menu_command(cmd) {
            return;
        }
        let (item, wins) = self.menu_ctx.take().unwrap_or((None, Vec::new()));
        match cmd {
            CMD_CONTROL_CENTER => self.toggle_control_center(),
            CMD_NEW => {
                if let Some(it) = &item {
                    apps::launch(&it.key);
                }
            }
            CMD_PIN => {
                if let Some(it) = item {
                    self.save_and_apply(|c| {
                        if it.pinned {
                            c.pins.retain(|p| p.app() != Some(it.key.as_str()));
                        } else {
                            c.pins.push(Pin::Id(it.key.clone()));
                        }
                    });
                }
            }
            CMD_ADD_SEP => {
                if let Some(it) = item {
                    self.save_and_apply(|c| {
                        if let Some(i) = c.pins.iter().position(|p| p.app() == Some(it.key.as_str())) {
                            c.pins.insert(i + 1, Pin::sep());
                        }
                    });
                }
            }
            CMD_CLOSE => wins.iter().for_each(|w| tracker::close(w.hwnd)),
            CMD_TRASH_OPEN => apps::launch(apps::RECYCLE_BIN),
            CMD_TASKMGR => apps::launch("C:\\Windows\\System32\\Taskmgr.exe"),
            CMD_WINSETTINGS => apps::launch("ms-settings:"),
            CMD_EXPLORER => apps::launch("Microsoft.Windows.Explorer"),
            CMD_TERMINAL => apps::launch("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"),
            CMD_AUTOHIDE => self.save_and_apply(|c| c.auto_hide = !c.auto_hide),
            CMD_MAGNIFY => self.save_and_apply(|c| c.magnify = !c.magnify),
            CMD_TASKBAR => self.save_and_apply(|c| c.hide_windows_taskbar = !c.hide_windows_taskbar),
            CMD_CLOCK_24H => self.save_and_apply(|c| c.clock_12h = !c.clock_12h),
            CMD_CLOCK_DATE => self.save_and_apply(|c| c.clock_date = !c.clock_date),
            CMD_CLOCK_SECONDS => self.save_and_apply(|c| c.clock_seconds = !c.clock_seconds),
            CMD_CLOCK_HIDE => self.save_and_apply(|c| c.desktop_clock = false),
            // The way back: the clock card is created by the panel as soon as the switch is on.
            CMD_CLOCK_DESKTOP => self.save_and_apply(|c| c.desktop_clock = !c.desktop_clock),
            cmd if (CMD_CLOCK_CORNER_BASE..CMD_CLOCK_CORNER_BASE + 4).contains(&cmd) => {
                let corner = crate::config::ClockCorner::from_code(cmd - CMD_CLOCK_CORNER_BASE);
                // A corner is a preset: it drops wherever the card was dragged to.
                self.save_and_apply(|c| {
                    c.clock_corner = corner;
                    c.clock_pos = None;
                });
            }
            CMD_CLOCK_LOCK => self.save_and_apply(|c| c.clock_locked = !c.clock_locked),
            CMD_SYSTEM_DESKTOP => self.save_and_apply(|c| c.desktop_system = !c.desktop_system),
            CMD_SYSTEM_HIDE => self.save_and_apply(|c| c.desktop_system = false),
            CMD_SYSTEM_LOCK => self.save_and_apply(|c| c.system_locked = !c.system_locked),
            cmd if (CMD_SYSTEM_CORNER_BASE..CMD_SYSTEM_CORNER_BASE + 4).contains(&cmd) => {
                let corner = crate::config::ClockCorner::from_code(cmd - CMD_SYSTEM_CORNER_BASE);
                // A corner is a preset: it drops wherever the card was dragged to.
                self.save_and_apply(|c| {
                    c.system_corner = corner;
                    c.system_pos = None;
                });
            }
            cmd if (CMD_SYSTEM_SCALE_BASE..CMD_SYSTEM_SCALE_BASE + CLOCK_SCALES.len()).contains(&cmd) => {
                let scale = CLOCK_SCALES[cmd - CMD_SYSTEM_SCALE_BASE];
                self.save_and_apply(|c| c.system_scale = scale);
            }
            cmd if (CMD_CLOCK_SCALE_BASE..CMD_CLOCK_SCALE_BASE + CLOCK_SCALES.len()).contains(&cmd) => {
                let scale = CLOCK_SCALES[cmd - CMD_CLOCK_SCALE_BASE];
                self.save_and_apply(|c| c.clock_scale = scale);
            }
            CMD_WIDGET_ADD => {
                if !self.launchpad.open {
                    self.open_launchpad();
                }
                self.lp_act(Act::AddWidgets);
            }
            cmd if (CMD_WIDGET_TOGGLE_BASE..CMD_WIDGET_TOGGLE_BASE + DOCK_WIDGETS.len()).contains(&cmd) => {
                let id = DOCK_WIDGETS[cmd - CMD_WIDGET_TOGGLE_BASE];
                self.save_and_apply(|c| {
                    let present = c.pins.iter().any(|p| matches!(p, Pin::Widget { widget, .. } if widget.as_str() == id));
                    if present {
                        c.pins.retain(|p| !matches!(p, Pin::Widget { widget, .. } if widget.as_str() == id));
                    } else {
                        c.pins.push(Pin::Widget { widget: id.to_string(), size: None });
                    }
                });
            }
            CMD_SETTINGS => {
                // The Settings view lives in Launchpad's centre panel.
                if !self.launchpad.open {
                    self.open_launchpad();
                }
                self.lp_act(Act::OpenSettings);
            }
            CMD_UPDATE_NOW => match crate::updater::begin_install() {
                Ok(()) => unsafe {
                    // Velopack's updater waits for this process to exit, installs, and
                    // starts the new version. Quit cleanly so the taskbar is restored meanwhile.
                    trace!("installing update: quitting so the updater can replace files");
                    self.shutdown();
                    let _ = DestroyWindow(self.hwnd);
                },
                Err(e) => self.notice(&["Couldn't start the update".into(), e]),
            },
            CMD_UPDATE_NOTES => {
                if let Some(v) = &self.app_update {
                    apps::launch(&crate::updater::notes_url(v));
                }
            }
            CMD_UPDATE_TOMORROW => self.snooze_update(24 * 3600),
            CMD_UPDATE_WEEK => self.snooze_update(7 * 24 * 3600),
            CMD_UPDATE_SKIP => {
                let v = self.app_update.clone().unwrap_or_default();
                self.app_update_manual = false;
                self.save_and_apply(|c| c.update_skip = v);
                self.refresh(false);
            }
            CMD_UPDATE_CHECK => self.check_for_update(true),
            CMD_QUIT => unsafe {
                // Clean up here: WM_DESTROY can't reach the dock while this handler holds it.
                self.shutdown();
                let _ = DestroyWindow(self.hwnd);
            },
            CMD_LP_OPEN | CMD_LP_ADMIN | CMD_LP_DOCK | CMD_LP_PIN | CMD_LP_REVEAL => {
                if let Some(id) = self.lp_menu_app.take() {
                    match cmd {
                        CMD_LP_OPEN => {
                            self.close_launchpad();
                            self.open_app(&id);
                        }
                        CMD_LP_ADMIN => {
                            self.close_launchpad();
                            apps::launch_as_admin(&id);
                        }
                        CMD_LP_DOCK => {
                            let pinned = self.cfg.pins.iter().any(|p| p.app() == Some(id.as_str()));
                            self.save_and_apply(|c| {
                                if pinned {
                                    c.pins.retain(|p| p.app() != Some(id.as_str()));
                                } else {
                                    c.pins.push(Pin::Id(id.clone()));
                                }
                            });
                        }
                        CMD_LP_PIN => {
                            let current = self.lp_pinned();
                            self.save_and_apply(|c| {
                                let mut list = current;
                                if let Some(i) = list.iter().position(|a| a == &id) {
                                    list.remove(i);
                                } else {
                                    list.push(id.clone());
                                }
                                c.launchpad_pins = list;
                            });
                            self.lp_render("all");
                        }
                        _ => {
                            if let Some(path) = self.pin_path(&id) {
                                self.close_launchpad();
                                apps::reveal(&path);
                            }
                        }
                    }
                }
            }
            CMD_POWER_LOCK => {
                self.close_launchpad();
                self.services.send(Req::Power(services::Power::Lock));
            }
            CMD_POWER_SIGNOUT => self.services.send(Req::Power(services::Power::SignOut)),
            CMD_POWER_SLEEP => {
                self.close_launchpad();
                self.services.send(Req::Power(services::Power::Sleep));
            }
            CMD_POWER_RESTART => self.services.send(Req::Power(services::Power::Restart)),
            CMD_POWER_SHUTDOWN => self.services.send(Req::Power(services::Power::ShutDown)),
            CMD_STACK_OPEN => {
                if let Some(n) = self.menu_stack.take() {
                    if let Some(i) = self.items.iter().position(|it| it.key == format!("::stack{n}")) {
                        self.open_stack(i);
                    }
                }
            }
            CMD_STACK_RENAME => {
                if let Some(n) = self.menu_stack.take() {
                    if let Some(i) = self.items.iter().position(|it| it.key == format!("::stack{n}")) {
                        self.open_stack(i);
                        if self.stack_view.open {
                            let c = self.compositor.clone();
                            self.stack_view.begin_rename(&c, &self.gfx);
                        }
                    }
                }
            }
            CMD_STACK_UNGROUP => {
                if let Some(n) = self.menu_stack.take() {
                    self.save_and_apply(|c| {
                        if let Some(Pin::Stack { apps, .. }) = c.pins.get(n).cloned() {
                            let existing: Vec<String> = c.pins.iter().filter_map(|p| p.app().map(String::from)).collect();
                            let add: Vec<Pin> = apps.into_iter().filter(|a| !existing.contains(a)).map(Pin::Id).collect();
                            c.pins.splice(n..n + 1, add);
                        }
                    });
                }
            }
            CMD_STACK_REMOVE => {
                if let Some(n) = self.menu_stack.take() {
                    self.save_and_apply(|c| {
                        if matches!(c.pins.get(n), Some(Pin::Stack { .. })) {
                            c.pins.remove(n);
                        }
                    });
                }
            }
            CMD_NEW_STACK => {
                if let Some(it) = item {
                    self.save_and_apply(|c| {
                        if let Some(i) = c.pins.iter().position(|p| p.app() == Some(it.key.as_str())) {
                            c.pins[i] = Pin::Stack { stack: "New Stack".into(), apps: vec![it.key.clone()] };
                        }
                    });
                }
            }
            c if c >= CMD_ADD_TO_STACK_BASE => {
                let n = c - CMD_ADD_TO_STACK_BASE;
                if let Some(it) = item {
                    self.save_and_apply(|cfg| {
                        if let Some(Pin::Stack { apps, .. }) = cfg.pins.get_mut(n) {
                            if !apps.contains(&it.key) {
                                apps.push(it.key.clone());
                            }
                        }
                        cfg.pins.retain(|p| p.app() != Some(it.key.as_str()));
                    });
                }
            }
            c if (CMD_WINDOW_BASE..CMD_WINDOW_BASE + 8).contains(&c) => {
                if let Some(w) = wins.get(c - CMD_WINDOW_BASE) {
                    tracker::activate(w.hwnd);
                }
            }
            c if (CMD_REMOVE_SEP_BASE..CMD_ADD_TO_STACK_BASE).contains(&c) => {
                let idx = c - CMD_REMOVE_SEP_BASE;
                self.save_and_apply(|cfg| {
                    if cfg.pins.get(idx).map(|p| p.is_sep()).unwrap_or(false) {
                        cfg.pins.remove(idx);
                    }
                });
            }
            _ => {}
        }
    }

    fn menu_choose(&mut self, cmd: Option<usize>) {
        self.menu.hide(true);
        self.menu_closed();
        if let Some(c) = cmd {
            self.on_menu_command(c);
        }
    }

    /// Messages for pop-up windows (menus, previews, stacks, Launchpad, Settings).
    fn popup_message(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if hwnd == self.mini_tray.hwnd() {
            use crate::tray_mini::Hit;
            let (x, y) = lparam_xy(lp);
            match msg {
                WM_MOUSEACTIVATE => return Some(LRESULT(MA_NOACTIVATE as isize)),
                WM_MOUSEMOVE => {
                    unsafe {
                        let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    let hit = self.mini_tray.at(x, y);
                    self.mini_tray.set_hover(hit);
                    let key = match hit { Some(Hit::Icon(key)) => Some(key), _ => None };
                    self.set_tray_hover(key, false);
                }
                WM_MOUSELEAVE => {
                    self.mini_tray.set_hover(None);
                    self.set_tray_hover(None, false);
                }
                WM_LBUTTONDOWN => {
                    self.mini_press = self.mini_tray.at(x, y);
                    if self.mini_press.is_some() { unsafe { SetCapture(hwnd); } }
                }
                WM_LBUTTONUP => {
                    let hit = self.mini_tray.at(x, y);
                    let pressed = self.mini_press.take();
                    unsafe { let _ = ReleaseCapture(); }
                    if hit.is_some() && hit == pressed {
                        match hit.unwrap() {
                            Hit::Icon(key) => {
                                // A real click gives our process foreground rights. The strip is
                                // otherwise non-activating; take foreground briefly so those rights
                                // can be delegated to the app before its callback is posted.
                                unsafe { let _ = SetForegroundWindow(hwnd); }
                                self.tray_click(key, crate::tray::Click::Left);
                            },
                            Hit::Overflow => if self.tray_popup.open { self.hide_tray_popup() } else { self.show_tray_popup() },
                            Hit::Control => self.toggle_control_center(),
                        }
                    }
                }
                WM_RBUTTONUP => {
                    if let Some(Hit::Icon(key)) = self.mini_tray.at(x, y) {
                        unsafe { let _ = SetForegroundWindow(hwnd); }
                        self.tray_click(key, crate::tray::Click::Right);
                    }
                }
                WM_CAPTURECHANGED => self.mini_press = None,
                _ => return None,
            }
            return Some(LRESULT(0));
        }
        if hwnd == self.tray_popup.hwnd() {
            let (x, y) = lparam_xy(lp);
            let bounds = self.tray_popup.bounds();
            let (screen_x, screen_y) = (bounds.left + x as i32, bounds.top + y as i32);
            match msg {
                WM_MOUSEACTIVATE => return Some(LRESULT(MA_NOACTIVATE as isize)),
                WM_MOUSEMOVE => {
                    unsafe {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    if self.tray_drag.is_some() {
                        self.tray_drag_move(screen_x, screen_y);
                    } else if let Some(press) = self.tray_press.as_ref() {
                        if press.origin == TrayOrigin::Popup
                            && ((screen_x - press.screen_x).pow(2) + (screen_y - press.screen_y).pow(2)) as f32 > (6.0 * self.scale).powi(2)
                        {
                            let key = press.key;
                            self.tray_drag_start(key, TrayOrigin::Popup);
                            self.tray_drag_move(screen_x, screen_y);
                        } else {
                            let key = self.tray_popup.cell_at(x, y);
                            self.set_tray_hover(key, true);
                        }
                    } else {
                        let key = self.tray_popup.cell_at(x, y);
                        self.set_tray_hover(key, true);
                    }
                }
                WM_MOUSELEAVE => {
                    let mut point = POINT::default();
                    let _ = get_cursor_pos(&mut point);
                    let inside = point.x >= bounds.left && point.x < bounds.right && point.y >= bounds.top && point.y < bounds.bottom;
                    if !inside && self.tray_drag.is_none() {
                        self.set_tray_hover(None, false);
                        if !self.mini_tray.contains(point.x, point.y) { self.hide_tray_popup(); }
                    }
                }
                WM_LBUTTONDOWN => {
                    if let Some(key) = self.tray_popup.cell_at(x, y) {
                        if self.tray_popup.stable_id(key).is_some() {
                            self.tray_press = Some(TrayPress { key, origin: TrayOrigin::Popup, screen_x, screen_y });
                            unsafe { SetCapture(hwnd); }
                        }
                    }
                }
                WM_LBUTTONUP => {
                    if self.tray_drag.is_some() {
                        self.tray_drag_move(screen_x, screen_y);
                        self.tray_drag_end(true);
                    } else if let Some(press) = self.tray_press.take() {
                        if self.tray_popup.cell_at(x, y) == Some(press.key) {
                            self.tray_click(press.key, crate::tray::Click::Left);
                        }
                    }
                    unsafe { let _ = ReleaseCapture(); }
                }
                WM_RBUTTONUP => {
                    if let Some(key) = self.tray_popup.cell_at(x, y) {
                        self.tray_click(key, crate::tray::Click::Right);
                    }
                }
                WM_CAPTURECHANGED => {
                    self.tray_press = None;
                    if self.tray_drag.as_ref().map(|drag| drag.origin == TrayOrigin::Popup).unwrap_or(false) {
                        self.tray_drag_end(false);
                    }
                }
                WM_KEYDOWN if wp.0 as u16 == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 => {
                    self.hide_tray_popup();
                }
                _ => return None,
            }
            return Some(LRESULT(0));
        }
        if hwnd == self.control_center.hwnd() {
            let (x, y) = lparam_xy(lp);
            match msg {
                WM_MOUSEMOVE => {
                    unsafe {
                        let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    if self.control_center.dragging() {
                        self.cc_drag(x, y);
                    } else {
                        let key = match self.control_center.at(x, y) {
                            Some((crate::control_center::CAct::Do(crate::widgets::WAct::Tray(key)), _, _)) => Some(key),
                            _ => None,
                        };
                        self.set_tray_hover(key, false);
                        if self.control_center.on_move(x, y) { self.cc_render(); }
                    }
                }
                WM_MOUSELEAVE => {
                    self.set_tray_hover(None, false);
                    if self.control_center.clear_hover() { self.cc_render(); }
                }
                WM_LBUTTONDOWN => {
                    let key = match self.control_center.at(x, y) {
                        Some((crate::control_center::CAct::Do(crate::widgets::WAct::Tray(key)), _, _)) => Some(key),
                        _ => None,
                    };
                    if let Some(key) = key {
                        self.tray_press = Some(TrayPress { key, origin: TrayOrigin::ControlCenter, screen_x: 0, screen_y: 0 });
                        unsafe { SetCapture(hwnd); }
                    } else {
                        self.cc_click(x, y);
                        if self.control_center.dragging() { unsafe { SetCapture(hwnd); } }
                    }
                }
                WM_LBUTTONUP => {
                    let pressed = self.tray_press.take().filter(|p| p.origin == TrayOrigin::ControlCenter).map(|p| p.key);
                    let released = match self.control_center.at(x, y) {
                        Some((crate::control_center::CAct::Do(crate::widgets::WAct::Tray(key)), _, _)) => Some(key),
                        _ => None,
                    };
                    self.control_center.end_drag();
                    unsafe { let _ = ReleaseCapture(); }
                    if let Some(key) = crate::tray_policy::matching_release(pressed, released) {
                        self.tray_click(key, crate::tray::Click::Left);
                    }
                }
                WM_CAPTURECHANGED => {
                    if self.tray_press.as_ref().is_some_and(|p| p.origin == TrayOrigin::ControlCenter) {
                        self.tray_press = None;
                    }
                    self.control_center.end_drag();
                }
                WM_RBUTTONUP => {
                    // Right-click on a tray icon in the footer: the app's own menu, as on the
                    // real taskbar.
                    let key = match self.control_center.at(x, y) {
                        Some((crate::control_center::CAct::Do(crate::widgets::WAct::Tray(key)), _, _)) => Some(key),
                        _ => None,
                    };
                    if let Some(key) = key {
                        self.tray_click(key, crate::tray::Click::Right);
                    }
                }
                WM_MOUSEWHEEL => {}
                WM_KEYDOWN => {
                    if wp.0 as u16 == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 {
                        self.close_control_center();
                    }
                }
                WM_ACTIVATE => {
                    if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE && self.control_center.open {
                        // Clicked elsewhere: close, without taking focus back from whatever it was.
                        self.close_control_center();
                    }
                }
                _ => return None,
            }
            return Some(LRESULT(0));
        }
        if hwnd == self.launchpad.hwnd() {
            let (x, y) = lparam_xy(lp);
            match msg {
                WM_MOUSEMOVE => {
                    if self.launchpad.dragging() {
                        self.lp_drag_move(x, y);
                    } else if let Some((src, px, py)) = self.lpw_press {
                        if ((x - px).powi(2) + (y - py).powi(2)).sqrt() > 6.0 * self.scale {
                            // Dragging a widget card or a gallery preview.
                            self.lpw_press = None;
                            unsafe {
                                let _ = KillTimer(Some(self.hwnd), T_LP_HOLD);
                            }
                            self.lp_begin_drag(src, px, py);
                            self.lp_drag_move(x, y);
                        }
                    } else if let Some(a) = self.lp_drag.clone() {
                        self.lp_slider(&a, x, false);
                    } else if let Some((id, px, py)) = self.lp_press.clone() {
                        if ((x - px).powi(2) + (y - py).powi(2)).sqrt() > 8.0 * self.scale {
                            // Dragging an app out of Launchpad: hand it to the dock.
                            self.lp_press = None;
                            self.close_launchpad();
                            self.start_external_drag(&id);
                        }
                    } else {
                        let tray = self.launchpad.hover_tray;
                        self.launchpad.on_move(&self.compositor, x, y);
                        if self.launchpad.hover_tray != tray {
                            self.lp_render("live");
                        }
                    }
                }
                WM_LBUTTONDOWN => {
                    if let Some(Act::App(id)) = self.launchpad.act_at(x, y) {
                        self.lp_press = Some((id, x, y));
                    }
                    match self.launchpad.act_at(x, y) {
                        Some(Act::Card(p) | Act::Widget(p, _, _)) => {
                            // Press: the card dips; holding lifts it, moving drags it.
                            self.lpw_press = Some((crate::launchpad::DragSrc::Col(p), x, y));
                            self.launchpad.press_card(&self.compositor, p);
                            unsafe {
                                SetCapture(hwnd);
                                SetTimer(Some(self.hwnd), T_LP_HOLD, 280, None);
                            }
                        }
                        Some(Act::GalleryItem(id)) => {
                            self.lpw_press = Some((crate::launchpad::DragSrc::Gallery(id), x, y));
                            unsafe {
                                SetCapture(hwnd);
                                SetTimer(Some(self.hwnd), T_LP_HOLD, 280, None);
                            }
                        }
                        _ => {}
                    }
                    if let Some(a) = self.launchpad.act_at(x, y) {
                        if matches!(a, Act::Slider(_)) {
                            self.lp_drag = Some(a.clone());
                            unsafe {
                                SetCapture(hwnd);
                            }
                            self.lp_slider(&a, x, false);
                        }
                    }
                }
                WM_LBUTTONUP => {
                    self.lp_press = None;
                    let was_pressed = self.lpw_press.take().is_some();
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), T_LP_HOLD);
                    }
                    if self.launchpad.dragging() {
                        self.lp_end_drag();
                    } else if let Some(a) = self.lp_drag.take() {
                        unsafe {
                            let _ = ReleaseCapture();
                        }
                        self.lp_slider(&a, x, true);
                    } else {
                        if was_pressed {
                            self.launchpad.release_card(&self.compositor);
                            unsafe {
                                let _ = ReleaseCapture();
                            }
                        }
                        if let Some(a) = self.launchpad.act_at(x, y) {
                            self.lp_act(a);
                        }
                    }
                }
                WM_RBUTTONUP => {
                    if let Some(Act::Widget(_, crate::widgets::WAct::Tray(key), _)) = self.launchpad.act_at(x, y) {
                        self.tray_click(key, crate::tray::Click::Right);
                    } else if let Some(Act::Card(p) | Act::Widget(p, _, _)) = self.launchpad.act_at(x, y) {
                        self.lp_widget_menu(p);
                    } else if let Some(Act::App(id)) = self.launchpad.act_at(x, y) {
                        let in_dock = self.cfg.pins.iter().any(|p| p.app() == Some(id.as_str()));
                        let in_lp = self.lp_pinned().contains(&id);
                        let has_path = self.pin_path(&id).is_some();
                        let name = self.names.get(&id).cloned().unwrap_or_else(|| id.clone());
                        let mut items = vec![
                            MenuItem::header(name),
                            MenuItem::item("Open", CMD_LP_OPEN),
                            MenuItem::item("Run as Administrator", CMD_LP_ADMIN),
                            MenuItem::sep(),
                            MenuItem::item("Keep in Dock", CMD_LP_DOCK).checked(in_dock),
                            MenuItem::item(if in_lp { "Unpin from Launchpad" } else { "Pin to Launchpad" }, CMD_LP_PIN),
                        ];
                        if has_path {
                            items.push(MenuItem::item("Open File Location", CMD_LP_REVEAL));
                        }
                        self.lp_menu_app = Some(id);
                        self.menu_ctx = None;
                        self.menu_open = true;
                        let mut p = POINT::default();
                        let _ = get_cursor_pos(&mut p);
                        let _ = self.menu.show(&self.compositor, &self.gfx, &self.backdrop, items, p, false, self.scale, self.monitor);
                    }
                }
                WM_MOUSEWHEEL => {
                    let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as f32 / 120.0;
                    // Wheel messages carry screen coordinates.
                    let (sx, sy) = ((lp.0 & 0xFFFF) as i16 as f32, ((lp.0 >> 16) & 0xFFFF) as i16 as f32);
                    let (wx, wy) = (sx - self.monitor.left as f32, sy - self.monitor.top as f32);
                    if let Some(ci) = self.launchpad.column_at(wx, wy) {
                        self.launchpad.on_wheel_col(ci, delta * 72.0 * self.scale);
                    } else {
                        self.launchpad.on_wheel(delta * 72.0 * self.scale);
                    }
                }
                WM_CHAR if self.launchpad.form.is_some() => {
                    let Some(ch) = self.launchpad.decode_char(wp.0 as u16) else { return Some(LRESULT(0)); };
                    let changed = if ch == '\u{16}' {
                        // Ctrl+V
                        clipboard_text().map(|t| self.launchpad.form_paste(&t)).unwrap_or(false)
                    } else {
                        self.launchpad.form_char(ch)
                    };
                    if changed {
                        self.lp_render("center");
                    }
                }
                WM_KEYDOWN if self.launchpad.form.is_some() => {
                    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_BACK, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_TAB};
                    let vk = wp.0 as u16;
                    if vk == VK_ESCAPE.0 {
                        self.launchpad.form = None;
                        self.lp_render("center");
                    } else if vk == VK_RETURN.0 {
                        self.save_form();
                    } else if vk == VK_TAB.0 {
                        self.launchpad.form_next(unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0);
                        self.lp_render("center");
                    } else if vk == VK_BACK.0 && self.launchpad.form_backspace() {
                        self.lp_render("center");
                    }
                }
                WM_CHAR => {
                    let Some(ch) = self.launchpad.decode_char(wp.0 as u16) else { return Some(LRESULT(0)); };
                    let changed = if ch == '\u{16}' {
                        clipboard_text().map(|t| self.launchpad.paste_query(&t)).unwrap_or(false)
                    } else {
                        self.launchpad.key_char(ch)
                    };
                    if changed { self.lp_query_changed(); }
                }
                WM_KEYDOWN => {
                    let vk = wp.0 as u16;
                    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP};
                    if vk == VK_ESCAPE.0 {
                        if self.launchpad.gallery.is_some() {
                            self.launchpad.gallery = None;
                            self.lp_render("all");
                        } else if self.launchpad.settings.is_some() {
                            self.launchpad.settings = None;
                            self.lp_render("all");
                        } else if !self.launchpad.query.is_empty() {
                            self.launchpad.query.clear();
                            self.lp_query_changed();
                        } else {
                            self.close_launchpad();
                        }
                    } else if vk == VK_RETURN.0 {
                        // Holding Enter must not repeat it (a first press arms Restart; a
                        // repeat would confirm it).
                        let repeat = (lp.0 >> 30) & 1 == 1;
                        if !repeat {
                            if let Some(a) = self.launchpad.selected_act() {
                                self.lp_act(a);
                            }
                        }
                    } else if self.launchpad.key_down(vk) {
                        self.lp_query_changed();
                    } else if vk == VK_DOWN.0 || vk == VK_UP.0 {
                        self.lp_render("all");
                    }
                }
                WM_ACTIVATE => {
                    // Clicking another window closes Launchpad (but not our own power menu).
                    if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE && !self.menu.open && !self.synthetic() && !testctl() {
                        self.close_launchpad();
                    }
                }
                _ => return None,
            }
            return Some(LRESULT(0));
        }
        if hwnd == self.stack_view.hwnd() {
            let (x, y) = lparam_xy(lp);
            let c = self.compositor.clone();
            match msg {
                WM_MOUSEMOVE => self.stack_view.on_move(&c, x, y),
                WM_LBUTTONDOWN | WM_RBUTTONDOWN if self.stack_view.outside(x, y) => {
                    // Clicked the invisible shadow margin: that's clicking away. Save a
                    // rename in progress and close, as for a click anywhere else.
                    let ev = self.stack_view.commit_rename(&c, &self.gfx);
                    self.stack_view.hide();
                    self.stack_event(ev);
                }
                WM_LBUTTONDOWN => {
                    let ev = self.stack_view.on_down(&c, &self.gfx, x, y);
                    unsafe {
                        SetCapture(hwnd); // keep getting the pointer while dragging an app
                    }
                    self.stack_event(ev);
                }
                WM_LBUTTONUP => {
                    let ev = self.stack_view.on_up(&c, x, y);
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    self.stack_event(ev);
                }
                WM_CAPTURECHANGED => self.stack_view.cancel_drag(&c),
                WM_CHAR => {
                    let ch = char::from_u32(wp.0 as u32).unwrap_or('\0');
                    self.stack_view.key_char(&c, &self.gfx, ch);
                }
                WM_KEYDOWN => {
                    let vk = wp.0 as u16;
                    let (handled, ev) = self.stack_view.key_down(&c, &self.gfx, vk);
                    if !handled && vk == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 {
                        self.stack_view.hide();
                    }
                    self.stack_event(ev);
                }
                WM_ACTIVATE => {
                    if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                        // Clicking away saves a rename in progress, like Finder.
                        let ev = self.stack_view.commit_rename(&c, &self.gfx);
                        self.stack_view.hide();
                        self.stack_event(ev);
                    }
                }
                _ => return None,
            }
            return Some(LRESULT(0));
        }
        if hwnd == self.preview.hwnd() {
            let (x, y) = lparam_xy(lp);
            match msg {
                WM_MOUSEACTIVATE => return Some(LRESULT(MA_NOACTIVATE as isize)),
                WM_MOUSEMOVE => {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), T_PREVIEW_HIDE);
                        let _ = KillTimer(Some(self.hwnd), T_HIDE);
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    self.preview.on_move(&self.compositor, x, y);
                }
                WM_MOUSELEAVE => unsafe {
                    SetTimer(Some(self.hwnd), T_PREVIEW_HIDE, 300, None);
                },
                WM_LBUTTONUP => match self.preview.on_click(x, y) {
                    Some(PreviewHit::Card(h)) => {
                        self.preview.hide();
                        tracker::activate(h);
                    }
                    Some(PreviewHit::Close(h)) => {
                        self.hide_preview();
                        tracker::close(h);
                    }
                    None => {}
                },
                _ => return None,
            }
            return Some(LRESULT(0));
        }
        if hwnd == self.menu.hwnd() {
            let (x, y) = lparam_xy(lp);
            match msg {
                WM_MOUSEMOVE => self.menu.on_move(&self.compositor, x, y),
                WM_LBUTTONUP | WM_RBUTTONUP => {
                    if let Some(cmd) = self.menu.on_click(x, y) {
                        self.menu_choose(Some(cmd));
                    }
                }
                WM_KEYDOWN => match self.menu.on_key(&self.compositor, wp.0 as u16) {
                    Some(cmd) => self.menu_choose(cmd),
                    None => {}
                },
                WM_ACTIVATE => {
                    if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE && self.menu.open {
                        // Clicked elsewhere: close without stealing focus back.
                        self.menu.hide(false);
                        self.menu_closed();
                    }
                }
                _ => return None,
            }
            return Some(LRESULT(0));
        }
        None
    }



    // ---------------------------------------------------------------- stacks

    fn open_stack(&mut self, i: usize) {
        let it = self.items[i].clone();
        let Some((pin, name, ids)) = self.stack_of(&it.key) else { return };
        self.hide_preview();
        let _ = self.show_label(None);
        let names: Vec<String> = ids.iter().map(|a| self.names.get(a).cloned().unwrap_or_else(|| a.clone())).collect();
        let brushes: Vec<Option<CompositionSurfaceBrush>> = ids.iter().map(|a| self.brushes.get(a).cloned()).collect();
        let list: Vec<StackApp> = ids
            .iter()
            .zip(names.iter())
            .zip(brushes.iter())
            .map(|((id, name), b)| StackApp { id, name, brush: b.as_ref() })
            .collect();
        let mut p = POINT::default();
        let _ = get_cursor_pos(&mut p);
        let cx_px = if self.hovering { self.anchor_x } else { (p.x - self.monitor.left) as f32 };
        let (cx, _sc) = self.geo.slot_center(cx_px, if self.hovering { 1.0 } else { 0.0 }, i);
        let edge = self.shelf_inner_y();
        let (c, gfx, bd) = (self.compositor.clone(), &self.gfx, self.backdrop.clone());
        let below = self.panel_top;
        if let Err(e) = self.stack_view.show(&c, gfx, &bd, pin, &name, &list, self.monitor.left + cx as i32, edge, below, self.scale, self.monitor) {
            trace!("stack failed: {e:?}");
        }
    }

    /// Act on what happened in the stack pop-up: open an app, save a new order or name.
    fn stack_event(&mut self, ev: Option<crate::stack::StackEvent>) {
        use crate::stack::StackEvent;
        let pin = self.stack_view.pin;
        match ev {
            None => {}
            Some(StackEvent::Open(id)) => {
                self.stack_view.hide();
                self.open_app(&id);
            }
            Some(StackEvent::Reorder(order)) => {
                trace!("stack {pin} reordered: {order:?}");
                self.save_and_apply(|c| {
                    if let Some(Pin::Stack { apps, .. }) = c.pins.get_mut(pin) {
                        // Same apps, new order (never drop or add any here).
                        if apps.len() == order.len() && order.iter().all(|a| apps.contains(a)) {
                            *apps = order;
                        }
                    }
                });
            }
            Some(StackEvent::Rename(name)) => {
                trace!("stack {pin} renamed: {name}");
                self.save_and_apply(|c| {
                    if let Some(Pin::Stack { stack, .. }) = c.pins.get_mut(pin) {
                        *stack = name;
                    }
                });
            }
        }
    }

    /// Open an app from a stack (or anywhere): bring it forward if running, else launch.
    fn open_app(&mut self, id: &str) {
        let wins = self.item_windows(&Item { kind: Kind::App, key: id.to_string(), pinned: true });
        match wins.first() {
            Some(w) => tracker::activate(w.hwnd),
            None => apps::launch(id),
        }
    }


    // ---------------------------------------------------------------- Launchpad

    /// Apps listed under "Pinned" in Launchpad.
    fn lp_pinned(&self) -> Vec<String> {
        if !self.cfg.launchpad_pins.is_empty() {
            return self.cfg.launchpad_pins.clone();
        }
        let mut v: Vec<String> = Vec::new();
        for p in &self.cfg.pins {
            match p {
                Pin::Id(id) if id != SEPARATOR => v.push(id.clone()),
                Pin::Stack { apps, .. } => v.extend(apps.iter().cloned()),
                _ => {}
            }
        }
        v.dedup();
        v.truncate(24);
        v
    }

    /// Make sure icon brushes exist for these app ids, using only the on-disk cache;
    /// anything not cached yet is fetched by the worker (tiles show placeholders meanwhile).
    fn lp_brushes(&mut self, ids: &[String]) {
        let px = (self.cfg.magnified_size.max(self.cfg.icon_size) * self.scale).ceil();
        let mut missing = Vec::new();
        for id in ids {
            if self.brushes.contains_key(id) {
                continue;
            }
            match self.icons.get_cached(&self.gfx.wic, id) {
                Some(src) => {
                    if let Ok(surface) = self.gfx.icon_surface(&src, px) {
                        if let Ok(b) = self.compositor.CreateSurfaceBrushWithSurface(&surface) {
                            let _ = b.SetStretch(CompositionStretch::Fill);
                            self.brushes.insert(id.clone(), b);
                        }
                    }
                }
                None => {
                    if self.icons_requested.insert(id.clone()) {
                        missing.push(id.clone());
                    }
                }
            }
        }
        if !missing.is_empty() {
            self.services.send(Req::Icons(self.icons.dir(), missing));
        }
    }

    fn lp_render(&mut self, which: &str) {
        if !self.launchpad.open {
            return;
        }
        let pinned = self.lp_pinned();
        let mut need = pinned.clone();
        if self.launchpad.all_open && self.launchpad.query.is_empty() {
            need.extend(self.lp_apps.iter().map(|a| a.id.clone()));
        }
        self.launchpad_results_ids(&mut need);
        self.lp_brushes(&need);
        for a in &self.lp_apps {
            self.names.entry(a.id.clone()).or_insert_with(|| a.name.clone());
        }
        let tray = self.tray_views();
        let wa = self.whatsapp_state();
        let tray_list: Vec<(u64, String)> = tray.iter().map(|t| (t.key, t.tip.clone())).collect();
        let app_version = crate::updater::version_label();
        let app_update = self.update_status_line();
        let app_update_ver = self.app_update.clone();
        let lp_wins = self.lp_windows();
        let data = lp::Data {
            apps: &self.lp_apps,
            brushes: &self.brushes,
            pinned: &pinned,
            names: &self.names,
            control: &self.control,
            stats: self.stats.as_ref(),
            weather: self.weather.as_ref(),
            files: &self.files,
            cfg: &self.cfg,
            user: &self.user_name,
            wifi: self.wifi.as_ref(),
            tray: &tray,
            tray_list: &tray_list,
            update: self.update.as_ref(),
            app_version: &app_version,
            app_update: &app_update,
            update_ui: Self::update_ui_for(&app_update_ver),
            hub: &self.hub,
            whatsapp: wa,
            windows: &lp_wins,
            history: &self.history,
        };
        self.launchpad.update_results(&data);
        let (c, gfx) = (self.compositor.clone(), &self.gfx);
        let r = match which {
            "all" => self.launchpad.render(&c, gfx, &data),
            "right" => self.launchpad.render_right_panel(&c, gfx, &data),
            "left" => self.launchpad.render_left_panel(&c, gfx, &data),
            "live" => self.launchpad.repaint_live(gfx, &data),
            "center" => self.launchpad.render_center_panel(&c, gfx, &data),
            _ => Ok(()),
        };
        if let Err(e) = r {
            trace!("launchpad render failed: {e:?}");
        }
    }

    /// Keep the strip above Windows' taskbar if it remains visible. The taskbar
    /// watchdog still owns its visibility; this window never modifies Explorer.
    fn mini_screen(&self) -> RECT {
        if self.taskbar_hidden { return self.monitor; }
        let p = POINT { x: self.monitor.right - 1, y: self.monitor.bottom - 1 };
        let m = unsafe { MonitorFromPoint(p, MONITOR_DEFAULTTOPRIMARY) };
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if unsafe { GetMonitorInfoW(m, &mut info) }.as_bool() { info.rcWork } else { self.monitor }
    }

    fn refresh_mini_tray(&mut self) {
        let icons: Vec<_> = self.tray_bar_keys.iter().filter_map(|key| self.tray_items.iter().find(|i| i.key == *key).cloned()).collect();
        let screen = self.mini_screen();
        if let Err(error) = self.mini_tray.show(&self.compositor, &self.gfx, &self.backdrop, &icons,
            !self.tray_overflow_keys.is_empty(), screen, self.scale) {
            trace!("mini tray could not be drawn: {error:?}");
        }
    }

    fn tray_dock_rect(&self, index: usize) -> Option<RECT> {
        if index >= self.items.len() || index >= self.geo.left.len() { return None; }
        let amount = if self.hovering && self.tray_drag.is_none() { 1.0 } else { 0.0 };
        let pointer_x = if amount > 0.0 { self.anchor_x } else { self.geo.center_abs(index) };
        let (center_x, magnification) = self.geo.slot_center(pointer_x, amount, index);
        let (top, bottom) = self.geo.item_box(magnification);
        let half = self.geo.b * magnification / 2.0;
        Some(RECT {
            left: self.monitor.left + (center_x - half).round() as i32,
            top: self.win_top() + top.round() as i32,
            right: self.monitor.left + (center_x + half).round() as i32,
            bottom: self.win_top() + bottom.round() as i32,
        })
    }

    fn tray_overflow_views(&self) -> Vec<crate::tray_strip::TrayIconView> {
        self.tray_overflow_keys.iter().filter_map(|key| self.tray_items.iter().find(|icon| icon.key == *key).cloned()).collect()
    }

    fn show_tray_popup(&mut self) {
        let Some(anchor) = self.mini_tray.overflow_rect() else {
            self.hide_tray_popup();
            return;
        };
        let icons = self.tray_overflow_views();
        if icons.is_empty() {
            self.hide_tray_popup();
            return;
        }
        self.close_launchpad();
        self.close_control_center();
        let _ = self.show_label(None);
        if self.tray_hover.map(|(_, popup)| popup).unwrap_or(false) {
            self.set_tray_hover(None, false);
        }
        let result = self.tray_popup.show(
            &self.compositor,
            &self.gfx,
            &self.backdrop,
            &icons,
            anchor,
            self.monitor,
            false,
            self.scale,
            self.cfg.panel_shadow,
        );
        if let Err(error) = result {
            trace!("tray overflow popup failed: {error:?}");
            self.tray_popup.hide();
        }
    }

    fn hide_tray_popup(&mut self) {
        if self.tray_hover.map(|(_, popup)| popup).unwrap_or(false) {
            self.set_tray_hover(None, false);
        } else {
            self.tray_popup.set_hover(None);
        }
        if self.tray_popup.open {
            self.tray_popup.hide();
        }
        self.tray_popup.set_drop_target(None);
    }

    fn set_tray_hover(&mut self, key: Option<u64>, in_popup: bool) {
        let next = key.map(|key| (key, in_popup));
        if self.tray_hover == next { return; }
        self.mini_tray.hide_tip();
        if let Some(open_key) = self.tray_custom_popup {
            if next.map(|(next_key, _)| next_key) != Some(open_key) {
                let mut point = POINT::default();
                let _ = get_cursor_pos(&mut point);
                crate::tray::send(open_key, crate::tray::TrayInput::PopupClose, point);
                self.tray_custom_popup = None;
            }
        }
        let previous = self.tray_hover;
        self.tray_hover = next;
        self.tray_tip_open = None;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_TRAY_TIP);
            if next.is_some() {
                SetTimer(Some(self.hwnd), T_TRAY_TIP, 500, None);
            }
        }
        match next {
            Some((key, true)) => {
                // A previous icon's delayed tooltip must not linger while the next one waits.
                self.tray_popup.reset_header(&self.compositor, &self.gfx);
                self.tray_popup.set_hover(Some(key));
            }
            Some((_, false)) => {
                if previous.map(|(_, popup)| popup).unwrap_or(false) {
                    self.tray_popup.reset_header(&self.compositor, &self.gfx);
                }
                self.tray_popup.set_hover(None);
            }
            None => {
                self.tray_popup.set_hover(None);
                self.tray_popup.reset_header(&self.compositor, &self.gfx);
            }
        }
        if let Some((key, _)) = next {
            let mut point = POINT::default();
            let _ = get_cursor_pos(&mut point);
            crate::tray::send(key, crate::tray::TrayInput::MouseMove, point);
        }
    }

    fn tray_tip_tick(&mut self) {
        unsafe { let _ = KillTimer(Some(self.hwnd), T_TRAY_TIP); }
        let Some((key, in_popup)) = self.tray_hover else { return };
        if self.tray_tip_open == Some(key) { return; }
        let Some(standard_tip) = self.tray_items.iter().find(|icon| icon.key == key).map(|icon| icon.standard_tip) else {
            self.set_tray_hover(None, false);
            return;
        };
        if standard_tip {
            if in_popup && self.tray_popup.open {
                self.tray_popup.show_tip(&self.compositor, &self.gfx, key);
            } else {
                let mut point = POINT::default();
                let _ = get_cursor_pos(&mut point);
                if self.mini_tray.contains(point.x, point.y) {
                    if let Some(icon) = self.tray_items.iter().find(|i| i.key == key) {
                        let _ = self.mini_tray.show_tip(&self.compositor, &self.gfx, &self.backdrop, key, &icon.tip, self.monitor, self.scale);
                    }
                }
            }
        } else {
            let mut point = POINT::default();
            let _ = get_cursor_pos(&mut point);
            crate::tray::send(key, crate::tray::TrayInput::PopupOpen, point);
            self.tray_custom_popup = Some(key);
        }
        self.tray_tip_open = Some(key);
    }

    fn tray_drag_start(&mut self, key: u64, origin: TrayOrigin) {
        let Some(icon) = self.tray_items.iter().find(|icon| icon.key == key) else { return };
        let stable_id = icon.stable_id.clone();
        self.tray_press = None;
        self.tray_drag = Some(TrayDrag { key, stable_id, origin, target: None });
        self.set_tray_hover(None, false);
        self.hide_preview();
        self.stack_view.hide();
        let _ = self.show_label(None);
        if origin == TrayOrigin::Dock {
            if let Some(index) = self.items.iter().position(|item| item.kind == Kind::Tray(key)) {
                let _ = self.slots[index].SetOpacity(0.28);
            }
        }
        let temp = Item { kind: Kind::Tray(key), key: format!("::tray:{key}"), pinned: false };
        self.items.push(temp);
        let ghost_result = self.show_ghost(self.items.len() - 1);
        self.items.pop();
        if let Err(error) = ghost_result {
            trace!("tray drag ghost failed: {error:?}");
        }
    }

    fn tray_drag_move(&mut self, screen_x: i32, screen_y: i32) {
        self.move_ghost(screen_x, screen_y);
        let Some(drag) = self.tray_drag.as_ref() else { return };
        let (key, origin) = (drag.key, drag.origin);
        let bounds = self.tray_popup.bounds();
        let in_popup = self.tray_popup.open
            && screen_x >= bounds.left && screen_x < bounds.right
            && screen_y >= bounds.top && screen_y < bounds.bottom;
        let mut popup_hover = None;
        let target = if in_popup {
            let x = screen_x - bounds.left;
            let y = screen_y - bounds.top;
            popup_hover = self.tray_popup.cell_at(x as f32, y as f32);
            if let Some(target_key) = popup_hover {
                let target_id = self.tray_popup.stable_id(target_key).map(str::to_owned);
                let target_rect = self.tray_popup.screen_rect(target_key);
                match (target_id, target_rect) {
                    (Some(stable_id), Some(rect)) if target_key != key => Some(TrayDrop::Relative {
                        stable_id,
                        after: screen_y > (rect.top + rect.bottom) / 2
                            || (screen_y == (rect.top + rect.bottom) / 2 && screen_x >= (rect.left + rect.right) / 2),
                        in_overflow: true,
                    }),
                    _ => None,
                }
            } else if origin == TrayOrigin::Dock {
                Some(TrayDrop::Hide)
            } else {
                None
            }
        } else {
            let x = (screen_x - self.monitor.left) as f32;
            let y = (screen_y - self.win_top()) as f32;
            match self.geo.hit(x, y, 0.0).and_then(|index| self.items.get(index).map(|item| (index, item))) {
                Some((index, item)) => match item.kind {
                    Kind::Tray(target_key) if target_key != key => {
                        let target_id = self.tray_items.iter().find(|icon| icon.key == target_key).map(|icon| icon.stable_id.clone());
                        target_id.map(|stable_id| {
                            let rect = self.tray_dock_rect(index).unwrap_or_default();
                            TrayDrop::Relative { stable_id, after: screen_x >= (rect.left + rect.right) / 2, in_overflow: false }
                        })
                    }
                    Kind::TrayOverflow if origin == TrayOrigin::Popup => Some(TrayDrop::Promote),
                    Kind::TrayOverflow => Some(TrayDrop::Hide),
                    _ => None,
                },
                None => None,
            }
        };
        self.tray_popup.set_hover(popup_hover);
        self.tray_popup.set_drop_target(popup_hover);
        if let Some(drag) = self.tray_drag.as_mut() {
            drag.target = target;
        }
        for (highlight_key, visual) in &self.tray_highlights {
            let active = match self.tray_drag.as_ref().and_then(|drag| drag.target.as_ref()) {
                Some(TrayDrop::Hide | TrayDrop::Promote) => *highlight_key == u64::MAX,
                Some(TrayDrop::Relative { stable_id, in_overflow: false, .. }) => {
                    self.tray_items.iter().any(|icon| icon.key == *highlight_key && icon.stable_id == *stable_id)
                }
                _ => false,
            };
            let _ = visual.SetOpacity(if active { 0.9 } else { 0.0 });
        }
    }

    fn tray_drag_end(&mut self, commit: bool) {
        let Some(drag) = self.tray_drag.take() else { return };
        self.hide_ghost(!commit);
        if let Some(index) = self.items.iter().position(|item| item.kind == Kind::Tray(drag.key)) {
            let _ = self.slots[index].SetOpacity(1.0);
        }
        self.tray_popup.set_drop_target(None);
        for (_, visual) in &self.tray_highlights { let _ = visual.SetOpacity(0.0); }
        if !commit {
            unsafe { let _ = PostMessageW(Some(self.hwnd), WM_APP_RESYNC_HOVER, WPARAM(0), LPARAM(0)); }
            return;
        }
        let Some(target) = drag.target else {
            unsafe { let _ = PostMessageW(Some(self.hwnd), WM_APP_RESYNC_HOVER, WPARAM(0), LPARAM(0)); }
            return;
        };
        let mut cfg = self.cfg.clone();
        // Include currently registered icons that have never been dragged before, preserving
        // their live (registration) order after any identities already saved by the user.
        for icon in &self.tray_items {
            if !cfg.tray_order.iter().any(|id| id == &icon.stable_id) {
                cfg.tray_order.push(icon.stable_id.clone());
            }
        }
        match target {
            TrayDrop::Relative { stable_id, after, in_overflow } => {
                crate::tray_strip::move_before(&mut cfg.tray_order, &drag.stable_id, &stable_id, after);
                if drag.origin == TrayOrigin::Dock && in_overflow {
                    if !cfg.tray_hidden.iter().any(|id| id == &drag.stable_id) { cfg.tray_hidden.push(drag.stable_id); }
                } else if drag.origin == TrayOrigin::Popup && !in_overflow {
                    cfg.tray_hidden.retain(|id| id != &drag.stable_id);
                }
            }
            TrayDrop::Hide => {
                if !cfg.tray_hidden.iter().any(|id| id == &drag.stable_id) { cfg.tray_hidden.push(drag.stable_id); }
            }
            TrayDrop::Promote => {
                cfg.tray_hidden.retain(|id| id != &drag.stable_id);
                if let Some(first) = self.tray_bar_keys.iter().filter_map(|key| self.tray_items.iter().find(|icon| icon.key == *key)).find(|icon| icon.stable_id != drag.stable_id) {
                    crate::tray_strip::move_before(&mut cfg.tray_order, &drag.stable_id, &first.stable_id, false);
                } else if !cfg.tray_order.iter().any(|id| id == &drag.stable_id) {
                    cfg.tray_order.insert(0, drag.stable_id);
                }
            }
        }
        self.save_and_apply(|current| {
            current.tray_order = cfg.tray_order;
            current.tray_hidden = cfg.tray_hidden;
        });
        unsafe { let _ = PostMessageW(Some(self.hwnd), WM_APP_RESYNC_HOVER, WPARAM(0), LPARAM(0)); }
    }

    /// Click a tray icon: Launchpad closes first so the app's window or menu can come up
    /// in front, at the pointer.
    fn tray_click(&mut self, key: u64, kind: crate::tray::Click) {
        // While our activatable popup is still foreground, let its recipient display
        // its own window or context menu; hiding the popup first can lose that right.
        crate::tray::prepare_click(key);
        self.close_launchpad();
        // Pop-ups are topmost. Hide both before an app opens its own menu or flyout.
        self.close_control_center();
        self.hide_tray_popup();
        self.set_tray_hover(None, false);
        let mut p = POINT::default();
        let _ = get_cursor_pos(&mut p);
        crate::tray::click(key, kind, p);
    }

    /// An app's tray icon as a brush. WIC gives icons straight (not premultiplied) alpha,
    /// which Direct2D can't draw directly, so convert first.
    fn tray_brush(&self, hicon: HICON, px: f32) -> Result<CompositionSurfaceBrush> {
        use windows::Win32::Graphics::Imaging::*;
        let wic = &self.gfx.wic;
        let bmp = unsafe { wic.CreateBitmapFromHICON(hicon)? };
        let conv = unsafe { wic.CreateFormatConverter()? };
        unsafe { conv.Initialize(&bmp, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)? };
        let src: IWICBitmapSource = conv.cast()?;
        let surface = self.gfx.icon_surface(&src, px)?;
        let b = self.compositor.CreateSurfaceBrushWithSurface(&surface)?;
        b.SetStretch(CompositionStretch::Fill)?;
        Ok(b)
    }

    /// All live tray icons, cached as composition brushes only when their picture changes.
    fn collect_tray_items(&mut self) -> Vec<crate::tray_strip::TrayIconView> {
        let snapshots = crate::tray::snapshots();
        let live: std::collections::HashSet<u64> = snapshots.iter().map(|i| i.key).collect();
        self.tray_brushes.retain(|key, _| live.contains(key));
        self.brushes.retain(|key, _| {
            !key.starts_with("::tray:") || key.strip_prefix("::tray:").and_then(|n| n.parse::<u64>().ok()).map(|id| live.contains(&id)).unwrap_or(false)
        });
        let px = (16.0 * self.scale).ceil();
        let mut out = Vec::new();
        for icon in snapshots {
            let Some(hicon) = icon.hicon else { continue };
            let fresh = self.tray_brushes.get(&icon.key).map(|(generation, _)| *generation == icon.generation).unwrap_or(false);
            if !fresh {
                match self.tray_brush(hicon, px) {
                    Ok(brush) => { self.tray_brushes.insert(icon.key, (icon.generation, brush)); }
                    Err(error) => {
                        trace!("tray icon {} ('{}') couldn't be drawn: {error:?}", icon.key, icon.tip);
                        if let Ok(brush) = self.compositor.CreateSurfaceBrush() {
                            self.tray_brushes.insert(icon.key, (icon.generation, brush));
                        }
                    }
                }
            }
            let brush = self.tray_brushes.get(&icon.key).map(|(_, brush)| brush.clone());
            let cache_key = format!("::tray:{}", icon.key);
            if let Some(brush) = &brush { self.brushes.insert(cache_key, brush.clone()); }
            out.push(crate::tray_strip::TrayIconView {
                key: icon.key,
                stable_id: icon.stable_id,
                tip: icon.tip,
                brush,
                app_hidden: icon.hidden,
                standard_tip: icon.standard_tip,
            });
        }
        out
    }

    /// Icons the Launchpad mirror should show (apps may keep icons in their own overflow).
    fn tray_views(&mut self) -> Vec<lp::TrayView> {
        let items = self.collect_tray_items();
        self.tray_items = items.clone();
        items.into_iter().filter(|icon| crate::tray_compact::show_icon(&icon.stable_id, icon.app_hidden)).map(|icon| lp::TrayView { key: icon.key, brush: icon.brush, tip: icon.tip }).collect()
    }

    fn launchpad_results_ids(&self, need: &mut Vec<String>) {
        if self.launchpad.query.is_empty() {
            return;
        }
        // Learning can lift an app from further down, so fetch icons for more than are shown.
        for (a, _) in laminar_sys::search::rank(&self.launchpad.query, &self.lp_apps, 40) {
            need.push(a.id.clone());
        }
        need.extend(self.launchpad.result_app_ids());
    }

    fn toggle_launchpad(&mut self) {
        if self.launchpad.open {
            self.close_launchpad();
        } else {
            self.open_launchpad();
        }
    }

    fn open_launchpad(&mut self) {
        self.close_control_center();
        self.hide_tray_popup();
        self.set_tray_hover(None, false);
        self.hide_preview();
        self.stack_view.hide();
        let _ = self.show_label(None);
        if self.tucked {
            self.set_tucked(false);
        }
        self.services.send(Req::Control);
        self.services.send(Req::Stats);
        // Windows Update status: at most every 20 minutes (each check takes seconds).
        if self.update_asked.map(|t| t.elapsed().as_secs() > 20 * 60).unwrap_or(true) {
            self.update_asked = Some(Instant::now());
            self.services.send(Req::Update);
        }
        if self.lp_apps.is_empty() {
            self.services.send(Req::Apps);
        }
        let pinned = self.lp_pinned();
        self.lp_brushes(&pinned);
        for a in &self.lp_apps {
            self.names.entry(a.id.clone()).or_insert_with(|| a.name.clone());
        }
        let reserve = self.geo.shelf_h + 2.0 * self.geo.margin;
        let tray = self.tray_views();
        let wa = self.whatsapp_state();
        let tray_list: Vec<(u64, String)> = tray.iter().map(|t| (t.key, t.tip.clone())).collect();
        let app_version = crate::updater::version_label();
        let app_update = self.update_status_line();
        let app_update_ver = self.app_update.clone();
        let lp_wins = self.lp_windows();
        let data = lp::Data {
            apps: &self.lp_apps,
            brushes: &self.brushes,
            pinned: &pinned,
            names: &self.names,
            control: &self.control,
            stats: self.stats.as_ref(),
            weather: self.weather.as_ref(),
            files: &self.files,
            cfg: &self.cfg,
            user: &self.user_name,
            wifi: self.wifi.as_ref(),
            tray: &tray,
            tray_list: &tray_list,
            update: self.update.as_ref(),
            app_version: &app_version,
            app_update: &app_update,
            update_ui: Self::update_ui_for(&app_update_ver),
            hub: &self.hub,
            whatsapp: wa,
            windows: &lp_wins,
            history: &self.history,
        };
        let (c, gfx, bd) = (self.compositor.clone(), &self.gfx, self.backdrop.clone());
        if let Err(e) = self.launchpad.show(&c, gfx, &bd, self.monitor, reserve, self.panel_top, self.scale, &data) {
            trace!("launchpad failed: {e:?}");
            return;
        }
        trace!(
            "launchpad dpi={} dock dpi={} system dpi={} monitor={}x{}",
            unsafe { GetDpiForWindow(self.launchpad.hwnd()) },
            unsafe { GetDpiForWindow(self.hwnd) },
            unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() },
            self.monitor.right - self.monitor.left,
            self.monitor.bottom - self.monitor.top
        );
        // Keep the dock above the Launchpad — but only where it is topmost to begin with.
        // A top panel is a plain window and the Launchpad is meant to cover it.
        unsafe {
            if !self.panel_top {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
            SetTimer(Some(self.hwnd), T_LP_TICK, 1000, None);
        }
        self.ind_state.iter_mut().for_each(|s| *s = IndState::default());
        self.update_indicators();
    }

    fn close_launchpad(&mut self) {
        if !self.launchpad.open {
            return;
        }
        self.launchpad.hide();
        self.files_gen.invalidate();
        self.files.clear();
        self.launchpad.wifi_open = false;
        self.lp_drag = None;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_LP_TICK);
            let _ = KillTimer(Some(self.hwnd), T_FILES);
            if self.cfg.auto_hide && !self.hovering {
                SetTimer(Some(self.hwnd), T_HIDE, self.cfg.hide_delay_ms.max(1), None);
            }
            // Re-check hover once Launchpad is gone, in case a leave notice was lost while it
            // was open (menus, settings, drags); otherwise the dock can stay up for good.
            let _ = PostMessageW(Some(self.hwnd), WM_APP_RESYNC_HOVER, WPARAM(0), LPARAM(0));
        }
    }

    fn lp_query_changed(&mut self) {
        // Invalidate before the debounce interval, not when the next request is sent.
        self.files_gen.invalidate();
        self.files.clear();
        self.lp_render("all");
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_FILES);
            if !self.launchpad.query.trim().is_empty() {
                SetTimer(Some(self.hwnd), T_FILES, 180, None);
            }
        }
    }

    fn lp_slider(&mut self, act: &Act, x: f32, commit: bool) {
        let Some(frac) = self.launchpad.slider_value(act, x) else { return };
        match act {
            Act::Slider(key) => {
                let Some((min, max, step)) = Launchpad::slider_range(key) else { return };
                let v = ((min + frac * (max - min)) / step).round() * step;
                let mut cfg = self.cfg.clone();
                lp::cfg_set(&mut cfg, key, v);
                if commit {
                    if let Err(error) = cfg.save() {
                        report_settings_error(self.hwnd, &error);
                    }
                }
                if cfg != self.cfg {
                    self.apply_config(cfg);
                }
                self.lp_render("all");
            }
            _ => {}
        }
    }

    fn lp_act(&mut self, act: Act) {
        trace!("launchpad action {act:?}");
        // Search learns what was picked for what was typed (not for a first "arm" press).
        if !matches!(act, Act::Arm(_) | Act::ClearHistory) {
            if let Some(key) = self.launchpad.result_key(&act) {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                self.history.record(&self.launchpad.query, &key, now);
                save_history(&self.history);
            }
        }
        match act {
            Act::Close => self.close_launchpad(),
            Act::App(id) => {
                self.close_launchpad();
                self.open_app(&id);
            }
            Act::File(path) => {
                self.close_launchpad();
                apps::launch(&path);
            }
            Act::Uri(u) => {
                self.close_launchpad();
                apps::launch(&u);
            }
            // From Settings → About: the result shows in place there, not as a menu.
            Act::CheckUpdates => self.check_for_update(false),
            Act::UpdateNow => self.on_menu_command(CMD_UPDATE_NOW),
            Act::UpdateNotes => self.on_menu_command(CMD_UPDATE_NOTES),
            Act::UpdateSnooze(secs) => {
                self.snooze_update(secs);
                self.lp_render_about();
            }
            Act::UpdateSkip => {
                self.on_menu_command(CMD_UPDATE_SKIP);
                self.lp_render_about();
            }
            Act::Web(q) => {
                self.close_launchpad();
                let enc: String = q
                    .bytes()
                    .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
                    .collect();
                apps::launch(&format!("https://www.bing.com/search?q={enc}"));
            }
            Act::ToggleAll => {
                self.launchpad.on_wheel(f32::MAX);
                self.launchpad.all_open = !self.launchpad.all_open;
                self.lp_render("all");
            }
            Act::OpenSettings => {
                self.launchpad.on_wheel(f32::MAX);
                self.launchpad.settings = Some(0);
                self.lp_render("all");
            }
            Act::CloseSettings => {
                self.launchpad.on_wheel(f32::MAX);
                self.launchpad.settings = None;
                self.lp_render("all");
            }
            Act::Section(k) => {
                self.launchpad.on_wheel(f32::MAX);
                self.launchpad.query.clear();
                self.launchpad.settings = Some(k);
                self.lp_query_changed();
            }
            Act::Choose(key, v) => {
                self.save_and_apply(|c| lp::cfg_set(c, key, v as f32));
                self.lp_render("all");
            }
            Act::Toggle(key) => {
                let v = lp::cfg_get(&self.cfg, key);
                self.save_and_apply(|c| lp::cfg_set(c, key, if v > 0.5 { 0.0 } else { 1.0 }));
                self.lp_render("all");
            }
            Act::Power => {
                let items = vec![
                    MenuItem::item("Lock", CMD_POWER_LOCK),
                    MenuItem::item("Sign out", CMD_POWER_SIGNOUT),
                    MenuItem::sep(),
                    MenuItem::item("Sleep", CMD_POWER_SLEEP),
                    MenuItem::item("Restart", CMD_POWER_RESTART),
                    MenuItem::item("Shut down", CMD_POWER_SHUTDOWN),
                ];
                let mut p = POINT::default();
                let _ = get_cursor_pos(&mut p);
                self.menu_ctx = None;
                self.menu_open = true;
                let _ = self.menu.show(&self.compositor, &self.gfx, &self.backdrop, items, p, !self.panel_top, self.scale, self.monitor);
            }
            Act::Wifi => {
                let on = !self.control.wifi.unwrap_or(false);
                self.control.wifi = Some(on);
                self.services.send(Req::SetWifi(on));
                if on && self.launchpad.wifi_open {
                    self.wifi = None;
                    self.services.send(Req::WifiList);
                }
                self.lp_render("all");
            }
            Act::WifiMenu => {
                self.launchpad.wifi_open = true;
                self.services.send(Req::WifiList);
                self.lp_render("all");
            }
            Act::WifiBack => {
                self.launchpad.wifi_open = false;
                self.lp_render("all");
            }
            Act::WifiJoin(ssid) => {
                trace!("wifi join {ssid}");
                self.services.send(Req::WifiConnect(ssid));
            }
            Act::Bluetooth => {
                let on = !self.control.bluetooth.unwrap_or(false);
                self.control.bluetooth = Some(on);
                self.services.send(Req::SetBluetooth(on));
                self.lp_render("all");
            }
            Act::Dark => {
                let on = !self.control.dark;
                self.control.dark = on;
                self.services.send(Req::SetDark(on));
                self.lp_render("all");
            }
            Act::Mute => {
                let m = !self.control.muted.unwrap_or(false);
                self.control.muted = Some(m);
                self.services.send(Req::SetMuted(m));
                self.lp_render("all");
            }
            Act::MediaPrev => self.services.send(Req::Media(services::Media::Previous)),
            Act::MediaPlay => self.services.send(Req::Media(services::Media::PlayPause)),
            Act::MediaNext => self.services.send(Req::Media(services::Media::Next)),
            Act::OpenConfig => {
                self.close_launchpad();
                apps::edit_file(&config::path());
            }
            Act::Snip => {
                self.close_launchpad();
                apps::launch("ms-screenclip:");
            }
            Act::Lock => {
                self.close_launchpad();
                self.services.send(Req::Power(services::Power::Lock));
            }
            Act::Slider(_) | Act::Search | Act::Card(_) | Act::GalleryItem(_) => {}
            Act::Widget(_, a, frac) => self.widget_act(a, frac),
            Act::AddWidgets => {
                self.launchpad.query.clear();
                self.launchpad.settings = None;
                self.launchpad.form = None;
                self.launchpad.gallery = Some(0);
                self.lp_query_changed();
            }
            Act::GalleryCat(k) => {
                self.launchpad.gallery = Some(k);
                self.lp_render("center");
            }
            Act::CloseGallery => {
                self.launchpad.gallery = None;
                self.lp_render("all");
            }
            Act::AddWidget(id, dock) => self.add_widget(id, dock),
            Act::RemoveWidget(id) => self.remove_lp_widget(id),
            Act::Connect(conn) => self.open_connect(conn),
            Act::FormFocus(k) => {
                if let Some(f) = self.launchpad.form.as_mut() {
                    f.focus = k;
                }
                self.lp_render("center");
            }
            Act::FormSave => self.save_form(),
            Act::FormCancel => {
                self.launchpad.form = None;
                self.lp_render("center");
            }
            Act::FormDisconnect => self.disconnect_form(),
            Act::Copy(text) => {
                set_clipboard_text(self.hwnd, &text);
                self.close_launchpad();
            }
            Act::Window(h) => {
                self.close_launchpad();
                tracker::activate(HWND(h as _));
            }
            Act::SetVolume(v) => {
                let f = v as f32 / 100.0;
                self.control.volume = Some(f);
                if v > 0 && self.control.muted == Some(true) {
                    self.control.muted = Some(false);
                    self.services.send(Req::SetMuted(false));
                }
                self.services.send(Req::SetVolume(f));
                self.lp_render("all");
            }
            Act::Run(target, args) => {
                self.close_launchpad();
                apps::run(&target, &args);
            }
            Act::Arm(cmd) => {
                self.launchpad.armed = Some(cmd);
                self.launchpad.armed_at = Instant::now();
                self.lp_render("center");
            }
            Act::Sys(cmd) => {
                use crate::launchpad::SysCmd;
                if cmd != SysCmd::EmptyBin {
                    // Only a deliberate second press confirms: not a double-click or a
                    // bounce, and only for the command that was armed.
                    let gap = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }.max(300) as u128;
                    if self.launchpad.armed != Some(cmd) || self.launchpad.armed_at.elapsed().as_millis() < gap {
                        return;
                    }
                }
                self.launchpad.armed = None;
                self.close_launchpad();
                match cmd {
                    SysCmd::Sleep => self.services.send(Req::Power(services::Power::Sleep)),
                    SysCmd::SignOut => self.services.send(Req::Power(services::Power::SignOut)),
                    SysCmd::Restart => self.services.send(Req::Power(services::Power::Restart)),
                    SysCmd::ShutDown => self.services.send(Req::Power(services::Power::ShutDown)),
                    SysCmd::EmptyBin => apps::empty_recycle_bin(),
                }
            }
            Act::ClearHistory => {
                self.history.clear();
                save_history(&self.history);
                self.lp_render("center");
            }
        }
    }

    /// Open windows for Launchpad's "switch to" results.
    fn lp_windows(&self) -> Vec<lp::WinRow> {
        if self.launchpad.query.trim().is_empty() {
            return Vec::new();
        }
        self.windows.iter().map(|w| lp::WinRow { hwnd: w.hwnd.0 as isize, title: w.title.clone(), app: w.app.clone() }).collect()
    }

    fn on_service_replies(&mut self) {
        let mut render = false;
        // Live data (stats, controls, update status) redraws cards in place, not all of Launchpad.
        let mut live = false;
        for r in self.services.take() {
            match r {
                Reply::Apps(list) => {
                    trace!("apps: {}", list.len());
                    self.lp_apps = list;
                    render = true;
                }
                Reply::Files(gen, query, hits) => {
                    if self.files_gen.accepts(gen, &query, &self.launchpad.query, self.launchpad.open) {
                        self.files = hits;
                        render = true;
                    }
                }
                Reply::Control(c) => {
                    let sample = self.opts.demo.as_deref() == Some("lp-wifi-sample");
                    self.control = c;
                    if sample {
                        self.control.wifi = Some(true);
                    }
                    live = true;
                }
                Reply::Wifi(_) if self.opts.demo.as_deref() == Some("lp-wifi-sample") => {}
                Reply::Wifi(w) => {
                    trace!("wifi: {:?}", w.as_ref().map(|v| v.len()));
                    self.wifi = Some(w);
                    render = true;
                }
                Reply::Update(u) => {
                    trace!("windows update: {u:?}");
                    self.update = Some(u);
                    live = true;
                }
                Reply::Stats(st) => {
                    self.hub.push_stats(&st);
                    self.stats = Some(st);
                    self.paint_dock_widgets(false);
                    live = true;
                }
                Reply::Weather(gen, lat, lon, w) => {
                    if !self.services.weather_is_current(gen)
                        || lat.to_bits() != self.cfg.weather_lat.to_bits()
                        || lon.to_bits() != self.cfg.weather_lon.to_bits() {
                        continue;
                    }
                    if w.is_some() {
                        self.weather = w;
                        self.paint_dock_widgets(true);
                    } else {
                        trace!("weather: no data");
                    }
                    // A first failure needs a retry too, not only failures after a timer tick.
                    let next = if self.weather.is_none() { 3 * 60 * 1000 } else { 30 * 60 * 1000 };
                    unsafe { SetTimer(Some(self.hwnd), T_WEATHER, next, None); }
                    render = true;
                }
                Reply::Icons(list) => {
                    for (id, name) in list {
                        self.icons.set_name(&id, &name);
                        self.names.entry(id).or_insert(name);
                    }
                    self.icons.flush();
                    render = true;
                }
            }
        }
        // The control centre follows the same replies (volume, radios, networks, media).
        self.cc_render();
        let busy = self.lp_drag.is_some() || self.launchpad.dragging();
        if render && self.launchpad.open && !busy {
            self.lp_render("all");
        } else if live && self.launchpad.open && !busy {
            self.lp_render("live");
            if self.launchpad.wifi_open {
                self.lp_render("right");
            }
        }
    }


    // ---------------------------------------------------------------- files dropped on the dock

    fn set_drop_highlight(&mut self, t: Option<usize>) {
        if t == self.drop_target {
            return;
        }
        let b = self.geo.b;
        for (idx, scale) in [(self.drop_target, 1.0f32), (t, 1.15f32)] {
            if let Some(Some(tile)) = idx.and_then(|i| self.tiles.get(i)) {
                let _ = tile.SetCenterPoint(v3(b / 2.0, b / 2.0, 0.0));
                if let Ok(o) = tile.cast::<CompositionObject>() {
                    let _ = crate::ui::animate_v3(&self.compositor, &o, "Scale", v3(scale, scale, 1.0), 150);
                }
            }
        }
        self.drop_target = t;
    }

    fn on_drop_event(&mut self, ev: crate::filedrop::Event) -> crate::filedrop::Target {
        use crate::filedrop::{Event, Target};
        match ev {
            Event::Enter(sx, sy, files, alt) => {
                trace!("files dragged over the dock at ({sx}, {sy}) alt={alt}: {files:?}");
                let pinnable = !files.is_empty() && files.iter().all(|f| crate::filedrop::is_pinnable(f));
                self.file_drag = Some(FileDrag { files, pinnable, alt, insert: None });
                self.file_drag_over(sx, sy)
            }
            Event::Over(sx, sy, alt) => {
                if let Some(fd) = self.file_drag.as_mut() {
                    fd.alt = alt;
                }
                self.file_drag_over(sx, sy)
            }
            Event::Leave => {
                self.file_drag_end();
                self.on_leave();
                Target::None
            }
            Event::Drop(sx, sy, files) => {
                let t = self.file_drag_over(sx, sy);
                let insert = self.file_drag.as_ref().and_then(|f| f.insert);
                let on_item = self.drop_target;
                self.file_drag_end();
                self.hide_preview();
                trace!("files dropped at ({sx}, {sy}): {:?} insert={insert:?} item={on_item:?} {} files", t as u8, files.len());
                let result = match (t, on_item) {
                    (Target::Pin, _) if insert.is_some() => {
                        self.pin_files_at(insert.unwrap(), &files);
                        Target::Pin
                    }
                    (Target::App, Some(i)) => {
                        let it = self.items[i].clone();
                        // Packaged apps (ids with '!') are activated for the files; classic apps get paths.
                        let exe = if apps::is_path(&it.key) {
                            Some(it.key.clone())
                        } else if it.key.contains('!') {
                            None
                        } else {
                            self.pin_path(&it.key).or_else(|| self.windows.iter().find(|w| w.app == it.key).map(|w| w.exe.clone()))
                        };
                        let _ = self.bounce(i);
                        crate::filedrop::open_with(&it.key, exe, &files);
                        Target::App
                    }
                    (Target::Trash, _) => {
                        crate::filedrop::recycle(&files);
                        Target::Trash
                    }
                    _ => Target::None,
                };
                // The pointer is still over the dock: resume normal hover tracking.
                let win_top = self.win_top();
                self.hovering = false;
                self.on_move((sx - self.monitor.left) as f32, (sy - win_top) as f32);
                result
            }
        }
    }

    /// Files dragged over the dock: decide pin / open with / recycle, opening a gap to pin into.
    /// Programs, shortcuts and folders pin (icons part to make room). Documents open with the
    /// app they're dropped on, unless Alt is held, which pins them instead. Keeping the two
    /// modes apart means icons never shift under a document that's heading for an app.
    fn file_drag_over(&mut self, sx: i32, sy: i32) -> crate::filedrop::Target {
        use crate::filedrop::Target;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_HIDE);
            let _ = KillTimer(Some(self.hwnd), T_SHRINK);
        }
        if self.tucked {
            self.set_tucked(false);
        }
        if !self.hovering {
            // OLE runs its own mouse loop, so set up hover state here (no magnification while
            // dropping, so the icons hold still).
            self.hovering = true;
            // Tiles a drag opened stay up when it comes back to the dock.
            if !(self.preview.open && self.drag_preview) {
                self.hide_preview();
            }
            let _ = self.show_label(None);
            self.apply_region();
        }
        let _ = self.animate_amount(0.0, 120);
        let win_top = self.win_top();
        let (x, y) = ((sx - self.monitor.left) as f32, (sy - win_top) as f32);
        let Some(fd) = &self.file_drag else { return Target::None };
        let old = fd.insert;
        let w = self.geo.b + self.geo.g;
        let (target, insert, on_item) = {
            // Item centres as currently drawn (an open gap shifts them by half its width each way).
            let vc = |j: usize| {
                self.geo.center_abs(j)
                    + match old {
                        Some(k) if j >= k => w / 2.0,
                        Some(_) => -w / 2.0,
                        None => 0.0,
                    }
            };
            let n = self.items.len();
            let in_band = self.geo.in_shelf_band(y, 30.0 * self.scale);
            let mut out = (Target::None, None, None);
            let pin_mode = fd.pinnable || fd.alt;
            if in_band && !fd.files.is_empty() && n > 0 {
                let item = (0..n).find(|&j| (x - vc(j)).abs() < self.geo.width[j] / 2.0);
                match item {
                    Some(j) if !pin_mode && self.items[j].kind == Kind::App => out = (Target::App, None, Some(j)),
                    _ if pin_mode && x > vc(0) - w && x < vc(n - 1) + w => {
                        let boundary = self.pinned_boundary();
                        let k = (1..boundary).find(|&j| vc(j) > x).unwrap_or(boundary);
                        out = (Target::Pin, Some(k), None);
                    }
                    _ => {}
                }
            }
            out
        };
        if insert != old {
            if let Some(o) = old {
                self.set_slot_value(o, "Gap", 0.0);
            }
            if let Some(k) = insert {
                self.set_slot_value(k, "Gap", w);
            }
            if let Ok(o) = self.ptr.cast::<CompositionObject>() {
                let _ = crate::ui::animate_scalar(&self.compositor, &o, "Delta", if insert.is_some() { w } else { 0.0 }, (220.0 * slowmo()) as i64);
            }
            if let Some(fd) = self.file_drag.as_mut() {
                fd.insert = insert;
            }
            trace!("file drag: insert={insert:?}");
        }
        self.set_drop_highlight(on_item);
        let under = {
            let shift = |j: usize| match insert {
                Some(k) if j >= k => w / 2.0,
                Some(_) => -w / 2.0,
                None => 0.0,
            };
            let in_band = self.geo.in_shelf_band(y, 30.0 * self.scale);
            (0..self.items.len()).find(|&j| in_band && (x - self.geo.center_abs(j) - shift(j)).abs() < self.geo.width[j] / 2.0)
        };
        self.drag_hover(under);
        target
    }

    /// A drag moved over item `idx` (or off the icons). Resting on a running app opens its
    /// window tiles, like hovering does, so the drag can go on to a window.
    fn drag_hover(&mut self, idx: Option<usize>) {
        let idx = idx.filter(|&i| {
            let it = self.items[i].clone();
            it.kind == Kind::App && !self.item_windows(&it).is_empty()
        });
        if idx == self.drag_under {
            return;
        }
        self.drag_under = idx;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_DRAG_PREVIEW);
            match idx {
                Some(i) if self.preview.open && self.drag_preview => {
                    if self.preview.key != self.items[i].key {
                        self.show_preview(i);
                    }
                }
                Some(_) => {
                    SetTimer(Some(self.hwnd), T_DRAG_PREVIEW, 600, None);
                }
                None => {}
            }
        }
    }

    /// A drag over the window tiles: highlight the tile under it and, after a pause, bring
    /// that window forward (and close the tiles) so the drag can be dropped into it.
    fn on_preview_drag(&mut self, ev: crate::filedrop::Event) {
        use crate::filedrop::Event;
        let local = |hwnd: HWND, sx: i32, sy: i32| {
            let mut r = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut r);
            }
            ((sx - r.left) as f32, (sy - r.top) as f32)
        };
        let (sx, sy, drop) = match ev {
            Event::Enter(x, y, _, _) | Event::Over(x, y, _) => (x, y, false),
            Event::Drop(x, y, _) => (x, y, true),
            Event::Leave => {
                self.drag_switch = None;
                self.preview.on_move(&self.compositor, -1.0, -1.0);
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), T_DRAG_SWITCH);
                    SetTimer(Some(self.hwnd), T_PREVIEW_HIDE, 300, None);
                }
                return;
            }
        };
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_PREVIEW_HIDE);
            let _ = KillTimer(Some(self.hwnd), T_HIDE);
        }
        let (x, y) = local(self.preview.hwnd(), sx, sy);
        self.preview.on_move(&self.compositor, x, y);
        let card = self.preview.window_at(x, y);
        if drop {
            // Nothing can be dropped on a tile; take it as "show me this window".
            unsafe {
                let _ = KillTimer(Some(self.hwnd), T_DRAG_SWITCH);
            }
            self.drag_switch = None;
            if let Some(h) = card {
                self.hide_preview();
                tracker::activate_during_drag(h);
                if self.cfg.auto_hide && !self.hovering {
                    unsafe {
                        SetTimer(Some(self.hwnd), T_HIDE, self.cfg.hide_delay_ms.max(1), None);
                    }
                }
            }
            return;
        }
        if card != self.drag_switch {
            self.drag_switch = card;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), T_DRAG_SWITCH);
                if card.is_some() {
                    SetTimer(Some(self.hwnd), T_DRAG_SWITCH, 500, None);
                }
            }
        }
    }

    /// Close any gap opened for a file drag and forget the drag.
    fn file_drag_end(&mut self) {
        self.drag_under = None;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_DRAG_PREVIEW);
        }
        if let Some(fd) = self.file_drag.take() {
            if let Some(k) = fd.insert {
                self.set_slot_value(k, "Gap", 0.0);
                if let Ok(o) = self.ptr.cast::<CompositionObject>() {
                    let _ = crate::ui::animate_scalar(&self.compositor, &o, "Delta", 0.0, (220.0 * slowmo()) as i64);
                }
            }
        }
        self.set_drop_highlight(None);
    }

    /// Pin dropped files (programs, shortcuts, folders, documents) before slot `k`.
    fn pin_files_at(&mut self, k: usize, files: &[String]) {
        let now = self.pinned_order();
        let pos = now.iter().filter(|(j, _)| *j < k).count();
        let mut order: Vec<Pin> = now.into_iter().map(|(_, p)| p).collect();
        let mut added = 0;
        for f in files {
            let f = &crate::apps::stable_path(f);
            let already = self.cfg.pins.iter().any(|p| match p {
                Pin::Id(a) => a.eq_ignore_ascii_case(f),
                Pin::Stack { apps, .. } => apps.iter().any(|a| a.eq_ignore_ascii_case(f)),
                Pin::Widget { .. } => false,
            });
            if already {
                continue;
            }
            order.insert((pos + added).min(order.len()), Pin::Id(f.clone()));
            added += 1;
        }
        trace!("pinned {added} dropped files at slot {k}");
        if added > 0 {
            self.apply_pin_order(order);
        }
    }

    // ---------------------------------------------------------------- previews

    /// The pointer moved onto (or off) a dock item: schedule, switch or close previews.
    fn preview_hover(&mut self, idx: Option<usize>) {
        if self.drag.is_some() || self.menu_open {
            return;
        }
        let target = idx.filter(|&i| {
            let it = self.items[i].clone();
            it.kind == Kind::App && !self.item_windows(&it).is_empty()
        });
        unsafe {
            match target {
                Some(i) => {
                    let _ = KillTimer(Some(self.hwnd), T_PREVIEW_HIDE);
                    if self.preview.open {
                        if self.preview.key != self.items[i].key {
                            self.show_preview(i);
                        }
                    } else {
                        self.preview_pending = Some(i);
                        SetTimer(Some(self.hwnd), T_PREVIEW, 450, None);
                    }
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), T_PREVIEW);
                    self.preview_pending = None;
                    if self.preview.open {
                        SetTimer(Some(self.hwnd), T_PREVIEW_HIDE, 300, None);
                    }
                }
            }
        }
    }

    fn show_preview(&mut self, i: usize) {
        let it = self.items[i].clone();
        let wins = self.item_windows(&it);
        if wins.is_empty() {
            return;
        }
        let mut p = POINT::default();
        let _ = get_cursor_pos(&mut p);
        // Drags don't magnify the icons.
        let amount = if self.hovering && self.file_drag.is_none() { 1.0 } else { 0.0 };
        let local_x = if amount > 0.0 { self.anchor_x } else { (p.x - self.monitor.left) as f32 };
        let (cx, _sc) = self.geo.slot_center(local_x, amount, i);
        self.drag_preview = self.file_drag.is_some();
        let edge = self.shelf_inner_y();
        let below = self.panel_top;
        let brush = self.brushes.get(&it.key).cloned();
        let _ = self.show_label(None);
        if let Err(e) = self.preview.show(
            &self.compositor,
            &self.gfx,
            &self.backdrop,
            &it.key,
            &wins,
            brush.as_ref(),
            self.monitor.left + cx as i32,
            edge,
            below,
            self.scale,
            self.monitor,
        ) {
            trace!("preview failed: {e:?}");
        }
    }

    fn hide_preview(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_PREVIEW);
            let _ = KillTimer(Some(self.hwnd), T_PREVIEW_HIDE);
        }
        self.preview_pending = None;
        self.preview.hide();
    }

    // ---------------------------------------------------------------- drag & drop

    /// Items 1..boundary are the pinned ones (after the launch button).
    fn pinned_boundary(&self) -> usize {
        let mut b = 1;
        while b < self.items.len() && self.items[b].pinned && matches!(self.items[b].kind, Kind::App | Kind::Separator | Kind::Stack | Kind::Widget) {
            b += 1;
        }
        b
    }

    fn draggable(&self, i: usize) -> bool {
        let it = &self.items[i];
        it.kind == Kind::App || it.kind == Kind::Stack || it.kind == Kind::Widget || (it.kind == Kind::Separator && it.pinned)
    }

    fn set_slot_value(&self, slot: usize, prop: &str, v: f32) {
        if let Some(acc) = self.accs.get(slot) {
            if let Ok(o) = acc.cast::<CompositionObject>() {
                let _ = crate::ui::animate_scalar(&self.compositor, &o, prop, v, (220.0 * slowmo()) as i64);
            }
        }
    }

    fn set_delta(&self) {
        let Some(d) = &self.drag else { return };
        let gap = if d.insert.is_some() { d.width } else { 0.0 };
        let collapse = if d.ext.is_none() && (d.insert.is_some() || d.removing) { d.width } else { 0.0 };
        if let Ok(o) = self.ptr.cast::<CompositionObject>() {
            let _ = crate::ui::animate_scalar(&self.compositor, &o, "Delta", gap - collapse, (220.0 * slowmo()) as i64);
        }
    }

    fn start_drag(&mut self, i: usize) {
        self.hide_preview();
        self.stack_view.hide();
        trace!("drag start: item {i} {}", self.items[i].key);
        let width = self.geo.width[i];
        self.drag = Some(Drag { index: i, insert: None, removing: false, width, merge: None, merge_ready: false, ext: None, ext_widget: None });
        let _ = self.animate_amount(0.0, 120);
        let _ = self.show_label(None);
        let _ = self.slots[i].SetOpacity(0.0);
        if let Some(Some(ind)) = self.inds.get(i) {
            let _ = ind.SetOpacity(0.0);
        }
        if let Some(Some(att)) = self.atts.get(i) {
            let _ = att.vis.SetOpacity(0.0);
        }
        // Show the ghost (a floating copy of the icon) under the pointer.
        if let Err(e) = self.show_ghost(i) {
            trace!("ghost failed: {e:?}");
        }
    }

    /// Begin dragging an app that came from Launchpad (pinning it where it's dropped).
    fn start_external_drag(&mut self, id: &str) {
        trace!("external drag start: {id}");
        self.lp_brushes(&[id.to_string()]);
        let width = self.geo.b + self.geo.g;
        // Show the ghost using a temporary item description.
        self.items.push(Item { kind: Kind::App, key: id.to_string(), pinned: false });
        let tmp = self.items.len() - 1;
        let r = self.show_ghost(tmp);
        self.items.pop();
        if let Err(e) = r {
            trace!("ghost failed: {e:?}");
        }
        self.drag = Some(Drag { index: usize::MAX, insert: None, removing: false, width, merge: None, merge_ready: false, ext: Some(id.to_string()), ext_widget: None });
        let _ = self.animate_amount(0.0, 120);
        if self.tucked {
            self.set_tucked(false);
        }
        self.hovering = true;
        self.apply_region();
        unsafe {
            SetCapture(self.hwnd);
        }
    }

    fn show_ghost(&mut self, i: usize) -> Result<()> {
        let c = self.compositor.clone();
        if self.ghost.is_none() {
            let popup = crate::ui::Popup::new(&c, false)?;
            let body = c.CreateContainerVisual()?;
            popup.root.Children()?.InsertAtTop(&body)?;
            self.ghost = Some(Ghost { popup, body, size: 0 });
        }
        let it = self.items[i].clone();
        let s = self.scale;
        let icon = (self.geo.b * 1.12).round();
        let blur = (14.0 * s).round();
        let size = (icon + 2.0 * blur + 8.0 * s).ceil();
        let g = self.ghost.as_mut().unwrap();
        g.body.Children()?.RemoveAll()?;
        g.size = size as i32;
        g.body.SetSize(crate::ui::v2(size, size))?;
        g.body.SetOpacity(1.0)?;
        g.body.SetScale(crate::ui::v3(1.0, 1.0, 1.0))?;
        g.body.SetCenterPoint(crate::ui::v3(size / 2.0, size / 2.0, 0.0))?;
        if let Some(v) = self.wviews.get(&i).filter(|_| it.kind == Kind::Widget) {
            // Widgets: a copy of the widget (shrunk to fit the ghost window if it's wide).
            let k = (size / (v.w + 8.0 * s)).min(1.0);
            let sp = crate::ui::stretch_sprite(&c, &v.surface, v.w, v.h)?;
            sp.SetScale(crate::ui::v3(k, k, 1.0))?;
            sp.SetOffset(crate::ui::v3((size - v.w * k) / 2.0, (size - v.h * k) / 2.0, 0.0))?;
            g.body.Children()?.InsertAtTop(&sp)?;
        } else if it.kind == Kind::Separator {
            let (line, _, _) = crate::ui::rounded(&c, (3.0 * s).round(), icon * 0.7, 1.5 * s, crate::ui::theme::ink(0.8))?;
            line.SetOffset(crate::ui::v3((size - 3.0 * s) / 2.0, (size - icon * 0.7) / 2.0, 0.0))?;
            g.body.Children()?.InsertAtTop(&line)?;
        } else if let Some(brush) = self.brushes.get(&it.key) {
            let sh = self.gfx.shadow_surface(icon * 0.22, blur, 0.45)?;
            let sb = c.CreateSurfaceBrushWithSurface(&sh)?;
            sb.SetStretch(CompositionStretch::Fill)?;
            let nine = c.CreateNineGridBrush()?;
            nine.SetSource(&sb)?;
            nine.SetInsets(icon * 0.22 + blur + 2.0)?;
            let shadow = c.CreateSpriteVisual()?;
            shadow.SetBrush(&nine)?;
            shadow.SetSize(crate::ui::v2(icon + 2.0 * blur, icon + 2.0 * blur))?;
            shadow.SetOffset(crate::ui::v3((size - icon) / 2.0 - blur, (size - icon) / 2.0 - blur + 6.0 * s, 0.0))?;
            g.body.Children()?.InsertAtTop(&shadow)?;
            let tile = c.CreateSpriteVisual()?;
            tile.SetBrush(brush)?;
            tile.SetSize(crate::ui::v2(icon, icon))?;
            tile.SetOffset(crate::ui::v3((size - icon) / 2.0, (size - icon) / 2.0, 0.0))?;
            g.body.Children()?.InsertAtTop(&tile)?;
        }
        let mut p = POINT::default();
        let _ = get_cursor_pos(&mut p);
        let half = (size / 2.0) as i32;
        g.popup.show(RECT { left: p.x - half, top: p.y - half, right: p.x + half, bottom: p.y + half }, false);
        Ok(())
    }

    fn move_ghost(&self, sx: i32, sy: i32) {
        if let Some(g) = &self.ghost {
            let half = g.size / 2;
            unsafe {
                let _ = SetWindowPos(g.popup.hwnd, Some(HWND_TOPMOST), sx - half, sy - half, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    fn hide_ghost(&mut self, poof: bool) {
        let Some(g) = &self.ghost else { return };
        if poof {
            // Shrink and fade, then hide.
            if let Ok(o) = g.body.cast::<CompositionObject>() {
                let _ = crate::ui::animate_scalar(&self.compositor, &o, "Opacity", 0.0, 200);
                let _ = crate::ui::animate_v3(&self.compositor, &o, "Scale", crate::ui::v3(0.3, 0.3, 1.0), 220);
            }
            unsafe { SetTimer(Some(self.hwnd), T_GHOST, 240, None) };
        } else {
            g.popup.hide();
        }
    }

    /// Pointer moved while dragging (client coordinates of the dock window).
    fn drag_move(&mut self, x: f32, y: f32) {
        let mut p = POINT::default();
        let _ = get_cursor_pos(&mut p);
        self.move_ghost(p.x, p.y);
        let Some(d) = &self.drag else { return };
        let i = d.index;
        let external = d.ext.is_some();
        let pinned = !external && self.items[i].pinned;
        // Dragged far past the shelf, towards the middle of the screen: unpin it.
        let removing = pinned && if self.panel_top {
            y > self.geo.y0 + self.geo.shelf_h + 90.0 * self.scale
        } else {
            y < self.geo.y0 - 90.0 * self.scale
        };
        let is_app = if external { d.ext_widget.is_none() } else { self.items[i].kind == Kind::App };
        // Holding an app over the middle of another pinned app or a stack makes/extends a stack.
        if is_app && !removing && self.geo.in_shelf_band(y, 20.0 * self.scale) {
            let boundary = self.pinned_boundary();
            let over = (1..boundary).find(|&j| {
                j != i
                    && matches!(self.items[j].kind, Kind::App | Kind::Stack)
                    && (x - self.geo.center_abs(j)).abs() < self.geo.b * 0.28
            });
            let current = d.merge;
            if over != current {
                if let Some(t) = current {
                    if let Some(Some(tile)) = self.tiles.get(t) {
                        if let Ok(o) = tile.cast::<CompositionObject>() {
                            let _ = crate::ui::animate_v3(&self.compositor, &o, "Scale", crate::ui::v3(1.0, 1.0, 1.0), 150);
                        }
                    }
                }
                if let Some(dm) = self.drag.as_mut() {
                    dm.merge = over;
                    dm.merge_ready = false;
                }
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), T_MERGE);
                    if let Some(t) = over {
                        let wait = if self.items[t].kind == Kind::Stack { 150 } else { 500 };
                        SetTimer(Some(self.hwnd), T_MERGE, wait, None);
                    }
                }
            }
            if over.is_some() {
                return; // hold position while deciding whether to stack
            }
        }
        let Some(d) = &self.drag else { return };
        // Where would it land? Before the first pinned slot whose centre is right of the pointer.
        let boundary = self.pinned_boundary();
        let mut k = boundary;
        if !removing {
            for j in 1..boundary {
                if j != i && self.geo.center_abs(j) > x {
                    k = j;
                    break;
                }
            }
        }
        // Landing back where it started means "no change"; an external icon only lands
        // when the pointer is down at the dock.
        let over_dock = if self.panel_top {
            y > -40.0 * self.scale && y < self.geo.y0 + self.geo.shelf_h + 70.0 * self.scale
        } else {
            y > self.geo.y0 - 70.0 * self.scale && y < self.geo.win_h + 40.0 * self.scale
        };
        let insert = if removing || (pinned && (k == i || k == i + 1)) || (external && !over_dock) { None } else { Some(k) };
        let (old_insert, old_removing) = (d.insert, d.removing);
        if insert == old_insert && removing == old_removing {
            return;
        }
        if let Some(o) = old_insert {
            self.set_slot_value(o, "Gap", 0.0);
        }
        if let Some(n) = insert {
            self.set_slot_value(n, "Gap", d.width);
        }
        // Close the dragged icon's old space whenever it's going somewhere else.
        let collapse = if insert.is_some() || removing { d.width } else { 0.0 };
        if !external && i + 1 < self.accs.len() {
            self.set_slot_value(i + 1, "Collapse", collapse);
        }
        if let Some(g) = &self.ghost {
            if let Ok(o) = g.body.cast::<CompositionObject>() {
                let _ = crate::ui::animate_scalar(&self.compositor, &o, "Opacity", if removing { 0.55 } else { 1.0 }, 120);
            }
        }
        if let Some(d) = self.drag.as_mut() {
            d.insert = insert;
            d.removing = removing;
        }
        self.set_delta();
        trace!("drag: insert={insert:?} removing={removing}");
    }

    /// The pin strings (app ids and "|") of the pinned items, in dock order.
    fn pin_of(&self, j: usize) -> Pin {
        let it = &self.items[j];
        match it.kind {
            Kind::Separator => Pin::sep(),
            Kind::Stack => self.stack_of(&it.key).map(|(n, _, _)| self.cfg.pins[n].clone()).unwrap_or_else(Pin::sep),
            Kind::Widget => widget_pin(&it.key).and_then(|n| self.cfg.pins.get(n).cloned()).unwrap_or_else(Pin::sep),
            _ => Pin::Id(it.key.clone()),
        }
    }

    fn pinned_order(&self) -> Vec<(usize, Pin)> {
        (1..self.pinned_boundary()).map(|j| (j, self.pin_of(j))).collect()
    }

    fn end_drag(&mut self, commit: bool) {
        let Some(d) = self.drag.take() else { return };
        if let Some(id) = d.ext.clone() {
            self.end_external_drag(d, &id, commit);
            return;
        }
        let i = d.index;
        let it = self.items[i].clone();
        trace!("drag end: commit={commit} insert={:?} removing={}", d.insert, d.removing);
        if commit && d.removing && it.pinned {
            self.hide_ghost(true);
            let order: Vec<Pin> = self.pinned_order().into_iter().filter(|(j, _)| *j != i).map(|(_, s)| s).collect();
            self.apply_pin_order(order);
            return;
        }
        if let Some(t) = d.merge {
            if let Some(Some(tile)) = self.tiles.get(t) {
                if let Ok(o) = tile.cast::<CompositionObject>() {
                    let _ = crate::ui::animate_v3(&self.compositor, &o, "Scale", crate::ui::v3(1.0, 1.0, 1.0), 150);
                }
            }
        }
        if commit && d.merge_ready && it.kind == Kind::App {
            // Dropped onto an app or stack: make a stack / add to it.
            let t = d.merge.unwrap();
            self.hide_ghost(true);
            let target = self.pin_of(t);
            let mut order: Vec<Pin> = Vec::new();
            for (j, p) in self.pinned_order() {
                if j == i {
                    continue;
                }
                if j == t {
                    order.push(match target.clone() {
                        Pin::Stack { stack, mut apps } => {
                            if !apps.contains(&it.key) {
                                apps.push(it.key.clone());
                            }
                            Pin::Stack { stack, apps }
                        }
                        Pin::Id(a) => Pin::Stack { stack: "New Stack".into(), apps: vec![a, it.key.clone()] },
                        w @ Pin::Widget { .. } => w,
                    });
                } else {
                    order.push(p);
                }
            }
            self.apply_pin_order(order);
            return;
        }
        if commit {
            if let Some(k) = d.insert {
                self.hide_ghost(false);
                let dragged = self.pin_of(i);
                let without: Vec<(usize, Pin)> = self.pinned_order().into_iter().filter(|(j, _)| *j != i).collect();
                let pos = without.iter().filter(|(j, _)| *j < k).count();
                let mut order: Vec<Pin> = without.into_iter().map(|(_, s)| s).collect();
                order.insert(pos, dragged);
                self.apply_pin_order(order);
                return;
            }
        }
        // Cancel: put everything back.
        self.hide_ghost(false);
        if let Some(n) = d.insert {
            self.set_slot_value(n, "Gap", 0.0);
        }
        if i + 1 < self.accs.len() {
            self.set_slot_value(i + 1, "Collapse", 0.0);
        }
        if let Ok(o) = self.ptr.cast::<CompositionObject>() {
            let _ = crate::ui::animate_scalar(&self.compositor, &o, "Delta", 0.0, 220);
        }
        let _ = self.slots[i].SetOpacity(1.0);
        self.ind_state[i] = IndState::default();
        self.update_indicators();
    }

    fn end_external_drag(&mut self, d: Drag, id: &str, commit: bool) {
        trace!("external drag end: commit={commit} insert={:?} merge_ready={}", d.insert, d.merge_ready);
        // The drag set `hovering` without asking Windows for a leave notice, so the dock
        // would never hear the pointer go and never auto-hide. Re-check once the mouse
        // capture is released (a posted message runs after the button-up handling).
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_APP_RESYNC_HOVER, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = d.merge {
            if let Some(Some(tile)) = self.tiles.get(t) {
                if let Ok(o) = tile.cast::<CompositionObject>() {
                    let _ = crate::ui::animate_v3(&self.compositor, &o, "Scale", crate::ui::v3(1.0, 1.0, 1.0), 150);
                }
            }
        }
        let already = self.cfg.pins.iter().any(|p| p.app() == Some(id));
        if commit && d.merge_ready && d.merge.is_some() {
            self.hide_ghost(true);
            let t = d.merge.unwrap();
            let target = self.pin_of(t);
            let order: Vec<Pin> = self
                .pinned_order()
                .into_iter()
                .filter(|(_, p)| p.app() != Some(id))
                .map(|(j, p)| {
                    if j != t {
                        return p;
                    }
                    match target.clone() {
                        Pin::Stack { stack, mut apps } => {
                            if !apps.iter().any(|a| a == id) {
                                apps.push(id.to_string());
                            }
                            Pin::Stack { stack, apps }
                        }
                        Pin::Id(a) => Pin::Stack { stack: "New Stack".into(), apps: vec![a, id.to_string()] },
                        w @ Pin::Widget { .. } => w,
                    }
                })
                .collect();
            self.apply_pin_order(order);
            return;
        }
        if commit {
            if let Some(k) = d.insert {
                self.hide_ghost(false);
                let without: Vec<(usize, Pin)> = self.pinned_order().into_iter().filter(|(_, p)| d.ext_widget.is_some() || p.app() != Some(id)).collect();
                let pos = without.iter().filter(|(j, _)| *j < k).count();
                let mut order: Vec<Pin> = without.into_iter().map(|(_, p)| p).collect();
                order.insert(pos.min(order.len()), d.ext_widget.clone().unwrap_or_else(|| Pin::Id(id.to_string())));
                self.apply_pin_order(order);
                return;
            }
        }
        let _ = already;
        self.hide_ghost(true);
        if let Some(n) = d.insert {
            self.set_slot_value(n, "Gap", 0.0);
        }
        if let Ok(o) = self.ptr.cast::<CompositionObject>() {
            let _ = crate::ui::animate_scalar(&self.compositor, &o, "Delta", 0.0, 220);
        }
    }

    /// Save a new pin order (keeping pins that aren't currently shown, e.g. icon failed).
    fn apply_pin_order(&mut self, mut order: Vec<Pin>) {
        // Keep pinned apps that aren't shown right now (e.g. their icon failed to load).
        let shown: Vec<Pin> = order.clone();
        for p in &self.cfg.pins {
            if let Some(a) = p.app() {
                if !shown.contains(p) && !self.items.iter().any(|it| it.pinned && it.key == a) {
                    order.push(p.clone());
                }
            }
        }
        self.save_and_apply(|c| c.pins = order);
    }

    // ---------------------------------------------------------------- settings & system

    fn reserve_height(&self) -> i32 {
        (self.geo.shelf_h + 2.0 * self.geo.margin).ceil() as i32
    }

    /// Give control back before stopping the guard. A crash before the guard is
    /// stopped still leaves the independent guard able to restore the taskbar.
    fn release_taskbar(&mut self) {
        taskbar::restore(self.tray_original.take());
        self.taskbar_hidden = false;
        if let Some(mut guard) = self.taskbar_watchdog.take() {
            let _ = guard.kill();
            let _ = guard.wait();
        }
    }

    fn fail_taskbar_takeover(&mut self, reason: &str) {
        self.release_taskbar();
        self.cfg.hide_windows_taskbar = false;
        if let Err(error) = self.cfg.save() { trace!("cannot save taskbar fallback: {error}"); }
        trace!("taskbar takeover disabled: {reason}");
        let message = if crate::strings::russian() {
            format!("Не удалось обеспечить безопасное восстановление панели задач Windows. Панель Windows оставлена включённой.\n\n{reason}")
        } else {
            format!("The Windows taskbar cannot be restored safely. It has been left enabled.\n\n{reason}")
        };
        // Do not enter a modal message loop while the Dock's RefCell is borrowed.
        std::thread::spawn(move || {
            let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
            unsafe { let _ = MessageBoxW(None, windows::core::PCWSTR(message.as_ptr()), w!("AuraDock"), MB_OK | MB_ICONERROR); }
        });
    }

    fn check_taskbar_guard(&mut self) {
        if !self.taskbar_hidden { return; }
        let healthy = self.taskbar_watchdog.as_mut().is_some_and(|guard| matches!(guard.try_wait(), Ok(None)));
        if !healthy {
            self.fail_taskbar_takeover("watchdog exited or could not be checked");
            self.apply_system();
        }
    }

    fn apply_system(&mut self) {
        if !self.integrate {
            return;
        }
        // Hide (or give back) Explorer's taskbar *before* choosing the edge. The edge depends
        // on whether that taskbar really keeps the bottom one, and judging that while we were
        // still about to hide it made the panel jump to the top for a tick and leave its
        // reservation behind — the empty strip at the top the user saw afterwards.
        if !self.helper {
            if self.cfg.hide_windows_taskbar && !self.taskbar_hidden {
                let state = taskbar::tray_state();
                let original = taskbar::original_state(state);
                match taskbar::spawn_watchdog(original) {
                    Ok(guard) => {
                        self.taskbar_watchdog = Some(guard);
                        self.tray_original = taskbar::take_over(state);
                        self.taskbar_hidden = true;
                    }
                    Err(error) => self.fail_taskbar_takeover(&error.to_string()),
                }
            } else if !self.cfg.hide_windows_taskbar && self.taskbar_hidden {
                self.release_taskbar();
            }
        }
        // The edge follows the checkbox and nothing else: the panel must never sniff
        // Explorer's taskbar while running. "Hide the Windows Taskbar" on — the bottom edge is
        // free, the panel sits at the bottom; off — the taskbar keeps the bottom, so the panel
        // goes to the top of the monitor instead of sitting on top of it.
        let panel_top = !self.cfg.hide_windows_taskbar;
        if panel_top != self.panel_top {
            trace!("panel edge -> {}", if panel_top { "top" } else { "bottom" });
            self.panel_top = panel_top;
            // Land on the monitor's own edge first, so the rebuild below doesn't draw the
            // panel where the other edge used to be; the appbar refines it in a moment.
            self.floor_y = if panel_top { self.monitor.top } else { self.monitor.bottom };
            // The whole layout mirrors: shelf position, growth direction, pop-up anchors.
            if self.geo.win_w > 0.0 {
                if let Err(e) = self.rebuild() {
                    trace!("rebuild after the edge change failed: {e:?}");
                }
            }
        }
        if self.helper {
            let h = self.reserve_height();
            let assigned = taskbar::appbar(self.hwnd, self.monitor, h, !self.cfg.auto_hide, self.panel_top, &mut self.appbar_edge, &mut self.appbar_rect);
            self.use_floor(assigned);
            return;
        }
        self.helpers.sync(self.cfg.show_on_all_displays);
        let h = self.reserve_height();
        let assigned = taskbar::appbar(self.hwnd, self.monitor, h, !self.cfg.auto_hide, self.panel_top, &mut self.appbar_edge, &mut self.appbar_rect);
        self.use_floor(assigned);
    }

    /// Move the panel window into place and into its z-order band.
    ///
    /// The band follows the edge. A bottom panel is topmost on purpose: it lives in the strip
    /// Windows reserved for it, so it overlaps no window, and it has to stay visible over
    /// maximised ones. A top panel is a second taskbar rather than an overlay, so it goes into
    /// the ordinary band and sits *under every window* — maximised windows stop above the
    /// reserved strip anyway, and anything that does reach the panel keeps the click.
    fn place_window(&mut self) -> Result<()> {
        let (x, y, w, h) = (self.monitor.left, self.win_top(), self.geo.win_w as i32, self.geo.win_h as i32);
        let want_topmost = !self.panel_top;
        // Only band changes and the first placement touch the z-order; every later move keeps
        // the place it has, so resizing doesn't reshuffle the panel on top of anything.
        if self.band_topmost == want_topmost && self.z_placed {
            unsafe {
                SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER)?;
            }
            return Ok(());
        }
        unsafe {
            if want_topmost {
                SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE)?;
            } else {
                // Leave the topmost band first: Windows keeps a window there until it is told
                // otherwise, and a window that was topmost would keep floating over the work.
                if self.band_topmost {
                    let _ = SetWindowPos(self.hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
                match crate::desktop_clock::app_in_front() {
                    // Just under the app in front: below the work, above the desktop.
                    Some(front) => SetWindowPos(self.hwnd, Some(front), x, y, w, h, SWP_NOACTIVATE)?,
                    // Nothing in front (the desktop is): move without touching the z-order.
                    None => SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER)?,
                }
            }
        }
        self.band_topmost = want_topmost;
        self.z_placed = true;
        Ok(())
    }

    /// Screen y of the panel window's top edge. `floor_y` is the shelf's outer edge: the
    /// window hangs above it at the bottom of the screen and below it at the top.
    fn win_top(&self) -> i32 {
        if self.panel_top {
            self.floor_y
        } else {
            self.floor_y - self.geo.win_h as i32
        }
    }

    /// Screen y of the shelf's inner edge — the side pop-ups hang off.
    fn shelf_inner_y(&self) -> i32 {
        self.win_top() + (if self.panel_top { self.geo.y0 + self.geo.shelf_h } else { self.geo.y0 }) as i32
    }

    /// Move the panel onto the strip Windows reserved for it: the bottom edge normally, the
    /// top edge when Explorer's taskbar keeps the bottom one.
    fn use_floor(&mut self, assigned: Option<RECT>) {
        let floor = match assigned {
            Some(r) => if self.panel_top { r.top } else { r.bottom },
            None => if self.panel_top { self.monitor.top } else { self.monitor.bottom },
        };
        if floor == self.floor_y {
            return;
        }
        trace!("panel floor: {} -> {}", self.floor_y, floor);
        self.floor_y = floor;
        if self.geo.win_w <= 0.0 {
            return;
        }
        let _ = self.place_window();
        // Everything that hangs off the panel follows it.
        let anchor = self.cc_anchor();
        self.control_center.reposition(self.monitor, anchor, false);
    }

    /// The hover label: a glass pill holding the label text, above everything else.
    fn build_label(&mut self) -> Result<()> {
        let (label, label_geom) = self.glass(0.66, 9.0 * self.scale)?;
        let _ = self.root.Children()?.Remove(&self.label);
        if let Ok(parent) = self.label_text.Parent() {
            let _ = parent.Children()?.Remove(&self.label_text);
        }
        label.Children()?.InsertAtTop(&self.label_text)?;
        label.SetOpacity(0.0)?;
        self.root.Children()?.InsertAtTop(&label)?;
        self.label = label;
        self.label_geom = label_geom;
        Ok(())
    }

    /// The glass brush: Windows' blur alone, or with extra blur on top.
    fn update_glass(&mut self) {
        self.backdrop = if self.cfg.extra_blur > 0.5 {
            match crate::effects::blurred_backdrop(&self.compositor, &self.host_backdrop, self.cfg.extra_blur * self.scale / 2.0) {
                Ok(b) => b,
                Err(e) => {
                    trace!("extra blur unavailable: {e:?}");
                    self.host_backdrop.clone()
                }
            }
        } else {
            self.host_backdrop.clone()
        };
    }

    /// Light or dark glass, from the setting and Windows' app mode. Returns true if it changed.
    fn update_theme(&mut self) -> bool {
        // LAMINAR_THEME=light|dark forces a theme (for testing without changing Windows).
        let forced = std::env::var("LAMINAR_THEME").ok();
        let light = match forced.as_deref() {
            Some("light") => true,
            Some("dark") => false,
            _ => self.cfg.match_windows_theme && !laminar_sys::theme::apps_use_dark(),
        };
        if light == crate::ui::theme::light() {
            return false;
        }
        trace!("theme: {}", if light { "light" } else { "dark" });
        crate::ui::theme::set_light(light);
        true
    }

    /// Redraw everything in the new theme (Windows switched light/dark, or the setting changed).
    ///
    /// Runs on the next tick after the switch, so it never rebuilds the panel under a click
    /// that is still being handled (that is what left the panel in pieces when the dark-mode
    /// tile was pressed). Everything that baked a colour is rebuilt against the new theme.
    fn retheme(&mut self) {
        // Icons and widget bitmaps were painted for the old theme.
        self.brushes.clear();
        self.update_glass();
        self.refresh(true);
        let _ = self.build_label();
        self.restart_clock();
        // The cards bake their colours when they draw: a theme flip has to start them again.
        self.restart_system();
        if self.launchpad.open {
            // Panels take their glass tint when they open: reopen with the new tint,
            // keeping the Launchpad Settings section the user was on.
            let settings = self.launchpad.settings;
            self.close_launchpad();
            self.open_launchpad();
            if let Some(sec) = settings {
                self.launchpad.settings = Some(sec);
                self.lp_render("all");
            }
        }
        self.paint_dock_widgets(true);
        self.apply_region();
        // Windows repaints Explorer on a theme flip, which can bring its taskbar back.
        if self.integrate && self.cfg.hide_windows_taskbar && self.taskbar_hidden {
            taskbar::set_visible(false);
        }
    }

    fn apply_config(&mut self, new: Config) {
        let old = std::mem::replace(&mut self.cfg, new);
        if old.weather_lat.to_bits() != self.cfg.weather_lat.to_bits() || old.weather_lon.to_bits() != self.cfg.weather_lon.to_bits() {
            self.weather = None; // Never show the previous location's weather under the new name.
            self.services.send(Req::Weather(self.cfg.weather_lat, self.cfg.weather_lon));
            self.paint_dock_widgets(true);
            unsafe { SetTimer(Some(self.hwnd), T_WEATHER, 3 * 60 * 1000, None); }
        }
        crate::winkey::set_enabled(self.cfg.win_key_launchpad);
        if old.start_at_login != self.cfg.start_at_login && self.is_main_dock() {
            if let Err(error) = crate::startup::set(self.cfg.start_at_login) {
                self.cfg.start_at_login = old.start_at_login;
                if let Err(save_error) = self.cfg.save() { report_settings_error(self.hwnd, &save_error); }
                self.notice(&["Не удалось изменить автозапуск Dock".into(), error]);
            }
        }
        let turned_on = !old.check_updates && self.cfg.check_updates;
        if (turned_on || old.update_check_hours != self.cfg.update_check_hours) && self.is_main_dock() {
            unsafe { SetTimer(Some(self.hwnd), T_UPDATE_CHECK, 2000, None) };
        }
        if old.extra_blur != self.cfg.extra_blur {
            self.update_glass();
            let _ = self.build_label();
        }
        if self.update_theme() {
            self.retheme();
        } else if old.visuals_differ(&self.cfg) {
            if old.magnified_size != self.cfg.magnified_size || old.magnify != self.cfg.magnify || old.icon_size != self.cfg.icon_size {
                self.brushes.clear(); // re-render icons at the new size
            }
            self.refresh(true);
        }
        if old.per_display_windows != self.cfg.per_display_windows {
            self.rescan();
        }
        if old.desktop_system != self.cfg.desktop_system
            || old.system_corner != self.cfg.system_corner
            || (old.system_scale - self.cfg.system_scale).abs() > f32::EPSILON
            || old.system_locked != self.cfg.system_locked
            || old.system_pos != self.cfg.system_pos
        {
            self.restart_system();
        }
        if old.desktop_clock != self.cfg.desktop_clock
            || crate::desktop_clock::Prefs::from_config(&old) != crate::desktop_clock::Prefs::from_config(&self.cfg)
        {
            self.restart_clock();
        }
        if old.auto_hide != self.cfg.auto_hide {
            if self.cfg.auto_hide {
                unsafe { SetTimer(Some(self.hwnd), T_HIDE, 800, None) };
            } else {
                self.set_tucked(false);
            }
        }
        self.apply_system();
        if self.is_main_dock() && !self.hidden_for_fullscreen { self.refresh_mini_tray(); }
    }

    /// The real dock on the main display (not a helper or test copy).
    fn is_main_dock(&self) -> bool {
        self.integrate && self.opts.monitor.is_none()
    }

    fn shutdown(&mut self) {
        self.mini_tray.hide();
        self.hub.shutdown();
        self.helpers.stop();
        // The desktop cards live in their own processes: they go with the panel.
        for host in [self.clock_host.take(), self.system_host.take()].into_iter().flatten() {
            let mut child = host;
            let _ = child.kill();
            let _ = child.wait();
        }
        crate::tray::stop();
        if self.integrate {
            taskbar::appbar(self.hwnd, self.monitor, 0, false, self.panel_top, &mut self.appbar_edge, &mut self.appbar_rect);
            if self.taskbar_hidden { self.release_taskbar(); }
        }
    }

    // ---------------------------------------------------------------- --measure

    fn measure_tick(&mut self) {
        let hwnd = self.hwnd;
        match self.measure_step {
            0 => {
                self.m_cpu0 = metrics::cpu_seconds();
                self.m_t0 = Instant::now();
                self.measure_step = 1;
                unsafe { SetTimer(Some(hwnd), T_MEASURE, 10_000, None) };
            }
            1 => {
                let wall = self.m_t0.elapsed().as_secs_f64();
                let cpu = metrics::cpu_seconds() - self.m_cpu0;
                let m = metrics::memory();
                self.report.push(("idle_seconds".into(), format!("{wall:.1}")));
                self.report.push(("idle_cpu_percent_of_one_core".into(), format!("{:.3}", cpu / wall * 100.0)));
                self.report.push(("working_set_mb".into(), format!("{:.1}", m.working_set)));
                self.report.push(("private_bytes_mb".into(), format!("{:.1}", m.private_bytes)));
                self.report.push(("task_manager_memory_mb".into(), format!("{:.1}", m.private_working_set)));
                self.m_cpu0 = metrics::cpu_seconds();
                self.m_t0 = Instant::now();
                self.sweep_t0 = Instant::now();
                self.measure_step = 2;
                unsafe {
                    SetTimer(Some(hwnd), T_SWEEP, 8, None);
                    SetTimer(Some(hwnd), T_MEASURE, 5_000, None);
                }
            }
            _ => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), T_SWEEP);
                    let _ = KillTimer(Some(hwnd), T_MEASURE);
                }
                let wall = self.m_t0.elapsed().as_secs_f64();
                let cpu = metrics::cpu_seconds() - self.m_cpu0;
                self.report.push(("hover_pointer_moves_per_second".into(), format!("{:.0}", self.sweep_moves as f64 / wall)));
                self.report.push(("hover_cpu_percent_of_one_core".into(), format!("{:.3}", cpu / wall * 100.0)));
                self.on_leave();
                let json = format!("{{\n{}\n}}", self.report.iter().map(|(k, v)| format!("  \"{k}\": {v}")).collect::<Vec<_>>().join(",\n"));
                println!("{json}");
                let _ = std::fs::write("laminar-metrics.json", &json);
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }

    fn sweep_tick(&mut self) {
        let t = self.sweep_t0.elapsed().as_secs_f32();
        let g = &self.geo;
        let x = g.x0 + g.shelf_w * (0.5 + 0.48 * (t * std::f32::consts::TAU / 2.5).sin());
        let y = g.y0 + g.shelf_h / 2.0;
        self.sweep_moves += 1;
        self.on_move(x, y);
    }
}

fn lparam_xy(lp: LPARAM) -> (f32, f32) {
    ((lp.0 & 0xFFFF) as i16 as f32, ((lp.0 >> 16) & 0xFFFF) as i16 as f32)
}

/// Registered message Explorer broadcasts when its taskbar is (re)created.
static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The "SHELLHOOK" message id: Windows tells shell windows about flashing buttons with it.
static SHELLHOOK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// wParam of a SHELLHOOK message when a window asks for attention.
const HSHELL_FLASH: usize = 0x8006;

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_TESTCTL if testctl() => {
            let (x, y) = lparam_xy(lp);
            if let Ok(mut g) = FAKE_CURSOR.lock() {
                *g = Some((x as i32, y as i32));
            }
            return LRESULT(0);
        }
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_NCHITTEST => {
            let (sx, sy) = lparam_xy(lp);
            let through = with_dock_ret(|d| {
                let top = d.win_top() as f32;
                d.headroom_passes_through(sx - d.monitor.left as f32, sy - top)
            })
            .unwrap_or(false);
            if through {
                return LRESULT(HTTRANSPARENT as isize);
            }
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_xy(lp);
            with_dock(|d| {
                let (screen_x, screen_y) = (d.monitor.left + x as i32, d.win_top() + y as i32);
                if d.tray_drag.is_some() {
                    d.tray_drag_move(screen_x, screen_y);
                    return;
                }
                if let Some(press) = d.tray_press.as_ref() {
                    if press.origin == TrayOrigin::Dock
                        && ((screen_x - press.screen_x).pow(2) + (screen_y - press.screen_y).pow(2)) as f32 > (6.0 * d.scale).powi(2)
                    {
                        let key = press.key;
                        d.tray_drag_start(key, TrayOrigin::Dock);
                        d.tray_drag_move(screen_x, screen_y);
                        return;
                    }
                }
                if d.drag.is_some() {
                    d.drag_move(x, y);
                    return;
                }
                if let Some((i, px, py)) = d.press {
                    if ((x - px).powi(2) + (y - py).powi(2)).sqrt() > 6.0 * d.scale && d.draggable(i) {
                        d.press = None;
                        d.start_drag(i);
                        d.drag_move(x, y);
                        return;
                    }
                }
                d.on_move(x, y)
            });
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            with_dock(|d| d.on_leave());
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_xy(lp);
            with_dock(|d| {
                d.pressed = d.geo.hit(x, y, 1.0);
                d.tray_press = None;
                if let Some(i) = d.pressed {
                    if let Kind::Tray(key) = d.items[i].kind {
                        if d.tray_items.iter().any(|icon| icon.key == key) {
                            d.tray_press = Some(TrayPress {
                                key,
                                origin: TrayOrigin::Dock,
                                screen_x: d.monitor.left + x as i32,
                                screen_y: d.win_top() + y as i32,
                            });
                        }
                        d.press = None;
                    } else {
                        d.press = Some((i, x, y));
                    }
                } else {
                    d.press = None;
                }
            });
            unsafe {
                SetCapture(hwnd); // keep receiving the pointer while dragging off the dock
            }
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_xy(lp);
            with_dock(|d| {
                d.press = None;
                if d.tray_drag.is_some() {
                    let (screen_x, screen_y) = (d.monitor.left + x as i32, d.win_top() + y as i32);
                    d.tray_drag_move(screen_x, screen_y);
                    d.tray_drag_end(true);
                } else if let Some(press) = d.tray_press.take() {
                    let hit = d.geo.hit(x, y, 1.0);
                    let same = hit.and_then(|i| match d.items.get(i).map(|item| item.kind) {
                        Some(Kind::Tray(key)) => Some(key),
                        _ => None,
                    }) == Some(press.key);
                    if same { d.tray_click(press.key, crate::tray::Click::Left); }
                } else if d.drag.is_some() {
                    d.end_drag(true);
                } else {
                    let hit = d.geo.hit(x, y, 1.0);
                    if hit.is_some() && hit == d.pressed {
                        d.ptr_xy = (x, y);
                        d.on_click(hit.unwrap());
                    }
                }
                d.pressed = None;
            });
            unsafe {
                let _ = ReleaseCapture();
            }
            return LRESULT(0);
        }
        WM_CAPTURECHANGED => {
            with_dock(|d| {
                d.press = None;
                d.tray_press = None;
                if d.tray_drag.as_ref().map(|drag| drag.origin == TrayOrigin::Dock).unwrap_or(false) {
                    d.tray_drag_end(false);
                }
                if d.drag.is_some() {
                    d.end_drag(false);
                }
            });
            return LRESULT(0);
        }
        WM_RBUTTONUP => {
            let (x, y) = lparam_xy(lp);
            with_dock(|d| d.context_menu(x, y, None));
            return LRESULT(0);
        }
        WM_APP_RESCAN => {
            // A window changed. If Explorer re-showed its taskbar, hide it again.
            let h = HWND(lp.0 as _);
            with_dock(|d| {
                if wp.0 as u32 == EVENT_OBJECT_SHOW && d.taskbar_hidden && tracker::is_explorer_taskbar(h) {
                    taskbar::set_visible(false);
                }
                if wp.0 as u32 == EVENT_SYSTEM_FOREGROUND {
                    d.fg = tracker::foreground();
                    // A game or a video went full screen: get out of its way.
                    d.update_fullscreen();
                    // Another program came to the front (from the dock, Alt+Tab, a
                    // notification...): leave Launchpad. Its own focus-loss message isn't
                    // enough, because Windows doesn't always give Launchpad focus to lose.
                    if d.launchpad.open && !d.synthetic() && !testctl() && !is_own_window(d.fg) {
                        d.close_launchpad();
                    }
                    if !is_own_window(d.fg) {
                        if d.tray_popup.open { d.hide_tray_popup(); }
                        if d.tray_hover.is_some() { d.set_tray_hover(None, false); }
                    }
                    d.attention.remove(&(d.fg.0 as isize)); // looked at: clear its dot
                    d.update_indicators();
                }
            });
            unsafe { SetTimer(Some(hwnd), T_RESCAN, 60, None) }; // debounce bursts of events
            return LRESULT(0);
        }
        WM_APP_SVC => {
            with_dock(|d| d.on_service_replies());
            return LRESULT(0);
        }
        crate::widgets::hub::WM_APP_WIDGETS => {
            with_dock(|d| d.on_widget_replies());
            return LRESULT(0);
        }
        WM_APP_WINKEY => {
            with_dock(|d| d.toggle_launchpad());
            return LRESULT(0);
        }
        WM_APP_CLOCK_MENU => {
            with_dock(|d| d.clock_menu());
            return LRESULT(0);
        }
        WM_APP_CLOCK_MOVED => {
            // The card posts the corner it was let go in, in physical px.
            let (x, y) = (wp.0 as i32, lp.0 as i32);
            with_dock(|d| d.save_and_apply(|c| c.clock_pos = Some([x, y])));
            return LRESULT(0);
        }
        WM_APP_SYSTEM_MENU => {
            with_dock(|d| d.system_menu());
            return LRESULT(0);
        }
        WM_APP_SYSTEM_MOVED => {
            let (x, y) = (wp.0 as i32, lp.0 as i32);
            with_dock(|d| d.save_and_apply(|c| c.system_pos = Some([x, y])));
            return LRESULT(0);
        }
        WM_APP_RESYNC_HOVER => {
            with_dock(|d| {
                if d.drag.is_none() {
                    d.resync_hover();
                }
            });
            return LRESULT(0);
        }
        WM_APP_UPDATE => {
            with_dock(|d| d.on_update_checked());
            return LRESULT(0);
        }
        WM_APP_APPBAR => {
            if wp.0 as u32 == windows::Win32::UI::Shell::ABN_POSCHANGED {
                // Do not call Shell while inside its callback; changes are coalesced.
                unsafe { SetTimer(Some(hwnd), T_APPBAR, 80, None) };
            }
            return LRESULT(0);
        }
        WM_APP_TRAY => {
            // Debounce bursts (apps commonly update picture + tooltip separately), then
            // rebuild the strip and any open overflow grid from one consistent snapshot.
            unsafe { SetTimer(Some(hwnd), T_TRAY_RENDER, 150, None) };
            return LRESULT(0);
        }
        WM_APP_CONFIG => {
            unsafe { SetTimer(Some(hwnd), T_CONFIG, 150, None) }; // editors often write twice
            return LRESULT(0);
        }
        WM_APP_SETTINGS => {
            dock_show_settings();
            return LRESULT(0);
        }
        WM_TIMER => {
            let id = wp.0;
            with_dock(|d| match id {
                T_SHRINK => unsafe {
                    let _ = KillTimer(Some(hwnd), T_SHRINK);
                    if !d.hovering {
                        d.apply_region();
                    }
                },
                T_RESCAN => unsafe {
                    let _ = KillTimer(Some(hwnd), T_RESCAN);
                    d.rescan();
                },
                T_REVEAL => d.on_reveal_timer(),
                T_APPBAR => unsafe {
                    let _ = KillTimer(Some(hwnd), T_APPBAR);
                    if d.integrate && d.appbar_edge.is_some() && !d.cfg.auto_hide {
                        let h = d.reserve_height();
                        let assigned = taskbar::appbar(d.hwnd, d.monitor, h, true, d.panel_top, &mut d.appbar_edge, &mut d.appbar_rect);
                        d.use_floor(assigned);
                    }
                },
                T_TRAY_RAISE => {
                    if crate::tray::active() {
                        crate::tray::on_taskbar_created();
                        crate::tray::raise();
                    } else { crate::tray::start(hwnd, WM_APP_TRAY); }
                },
                T_UPDATE_CHECK => unsafe {
                    // Look again every hour (no network; a check runs only when due).
                    SetTimer(Some(hwnd), T_UPDATE_CHECK, 3600 * 1000, None);
                    d.auto_update_tick();
                },
                T_TRAY_RENDER => unsafe {
                    let _ = KillTimer(Some(hwnd), T_TRAY_RENDER);
                    d.refresh(true);
                    if d.launchpad.open { d.lp_render("right"); }
                    if d.control_center.open { d.cc_render(); }
                },
                T_TRAY_TIP => d.tray_tip_tick(),
                T_FILES => unsafe {
                    let _ = KillTimer(Some(hwnd), T_FILES);
                    let query = d.launchpad.query.trim();
                    if d.launchpad.open && !query.is_empty() {
                        d.services.send(Req::Files(d.files_gen.current(), query.to_string()));
                    }
                },
                T_LP_TICK => {
                    // While Launchpad is open: refresh stats (and with them the clock).
                    d.services.send(Req::Stats);
                }
                T_WEATHER => {
                    d.services.send(Req::Weather(d.cfg.weather_lat, d.cfg.weather_lon));
                    // Nothing yet: try again in three minutes rather than in half an hour.
                    let next = if d.weather.is_none() { 3 * 60 * 1000 } else { 30 * 60 * 1000 };
                    unsafe {
                        let _ = SetTimer(Some(d.hwnd), T_WEATHER, next, None);
                    }
                }
                T_WIDGETS => {
                    d.check_taskbar_guard();
                    if d.retheme_pending {
                        d.retheme_pending = false;
                        d.retheme();
                    }
                    d.widget_tick();
                    // Some games go full screen without a foreground event.
                    d.update_fullscreen();
                }
                T_LP_HOLD => unsafe {
                    let _ = KillTimer(Some(hwnd), T_LP_HOLD);
                    // Held still: lift the card so it's clearly ready to move.
                    if let Some((src, px, py)) = d.lpw_press.take() {
                        if d.launchpad.open {
                            d.lp_begin_drag(src, px, py);
                            let mut pt = POINT::default();
                            let _ = get_cursor_pos(&mut pt);
                            d.lp_drag_move((pt.x - d.monitor.left) as f32, (pt.y - d.monitor.top) as f32);
                        }
                    }
                },
                T_PREVIEW => unsafe {
                    let _ = KillTimer(Some(hwnd), T_PREVIEW);
                    if let Some(i) = d.preview_pending.take() {
                        if d.hover == Some(i) {
                            d.show_preview(i);
                        }
                    }
                },
                T_DRAG_PREVIEW => unsafe {
                    let _ = KillTimer(Some(hwnd), T_DRAG_PREVIEW);
                    if let (Some(i), true) = (d.drag_under, d.file_drag.is_some()) {
                        trace!("drag rested on '{}': showing its windows", d.items[i].key);
                        d.show_preview(i);
                    }
                },
                T_DRAG_SWITCH => unsafe {
                    let _ = KillTimer(Some(hwnd), T_DRAG_SWITCH);
                    if let Some(h) = d.drag_switch.take() {
                        d.hide_preview();
                        let ok = tracker::activate_during_drag(h);
                        trace!("drag rested on a window tile: brought it forward = {ok}");
                        if d.cfg.auto_hide && !d.hovering {
                            SetTimer(Some(hwnd), T_HIDE, d.cfg.hide_delay_ms.max(1), None);
                        }
                    }
                },
                T_PREVIEW_HIDE => unsafe {
                    let _ = KillTimer(Some(hwnd), T_PREVIEW_HIDE);
                    let mut p = POINT::default();
                    let _ = get_cursor_pos(&mut p);
                    let under = WindowFromPoint(p);
                    if !d.synthetic() && under != d.preview.hwnd() && !(under == d.hwnd && d.hover.is_some()) {
                        d.preview.hide();
                        if d.cfg.auto_hide && !d.hovering {
                            SetTimer(Some(hwnd), T_HIDE, d.cfg.hide_delay_ms.max(1), None);
                        }
                    }
                },
                T_MERGE => unsafe {
                    let _ = KillTimer(Some(hwnd), T_MERGE);
                    let target = d.drag.as_ref().and_then(|dr| dr.merge);
                    if let Some(t) = target {
                        if let Some(dr) = d.drag.as_mut() {
                            dr.merge_ready = true;
                        }
                        if let Some(Some(tile)) = d.tiles.get(t) {
                            let b = d.geo.b;
                            let _ = tile.SetCenterPoint(v3(b / 2.0, b / 2.0, 0.0));
                            if let Ok(o) = tile.cast::<CompositionObject>() {
                                let _ = crate::ui::animate_v3(&d.compositor, &o, "Scale", v3(1.18, 1.18, 1.0), 180);
                            }
                        }
                        trace!("merge ready over item {t}");
                    }
                },
                T_GHOST => unsafe {
                    let _ = KillTimer(Some(hwnd), T_GHOST);
                    if let Some(g) = &d.ghost {
                        g.popup.hide();
                    }
                },
                T_TUCKED => unsafe {
                    let _ = KillTimer(Some(hwnd), T_TUCKED);
                    if d.tucked {
                        d.apply_region();
                    }
                },
                T_HIDE => unsafe {
                    let _ = KillTimer(Some(hwnd), T_HIDE);
                    if d.cfg.auto_hide && !d.hovering && !d.menu_open && !d.preview.open && d.drag.is_none() && !d.launchpad.open {
                        if d.stack_view.open || d.tray_popup.open {
                            // Stay up under an open stack or tray popup; check again until it closes.
                            SetTimer(Some(hwnd), T_HIDE, 250, None);
                        } else {
                            d.set_tucked(true);
                        }
                    } else if d.cfg.auto_hide && !d.tucked {
                        trace!(
                            "hide timer: staying up (hovering={} menu={} preview={} drag={} launchpad={})",
                            d.hovering,
                            d.menu_open,
                            d.preview.open,
                            d.drag.is_some(),
                            d.launchpad.open
                        );
                    }
                },
                T_CONFIG => unsafe {
                    let _ = KillTimer(Some(hwnd), T_CONFIG);
                    if let Some(c) = Config::load() {
                        if c != d.cfg {
                            d.apply_config(c);
                        }
                    }
                },
                T_MEASURE => d.measure_tick(),
                T_SWEEP => d.sweep_tick(),
                T_EXIT => unsafe {
                    d.shutdown();
                    let _ = DestroyWindow(hwnd);
                },
                _ => {}
            });
            return LRESULT(0);
        }
        WM_SETTINGCHANGE => {
            // "ImmersiveColorSet": Windows switched between light and dark mode.
            let area = if lp.0 != 0 { unsafe { windows::core::PCWSTR(lp.0 as *const u16).to_string().unwrap_or_default() } } else { String::new() };
            if area == "ImmersiveColorSet" {
                with_dock(|d| {
                    if d.update_theme() {
                        // Rebuilding the glass must not happen inside whatever click caused the
                        // switch: mark it and repaint on the next tick.
                        d.retheme_pending = true;
                    }
                });
            }
            return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            with_dock(|d| {
                let (m, dpi) = this_display(&d.opts);
                d.monitor = m;
                d.scale = if msg == WM_DPICHANGED { (wp.0 & 0xFFFF) as f32 / 96.0 } else { dpi as f32 / 96.0 };
                d.brushes.clear();
                d.tray_brushes.clear();
                d.refresh(true);
                d.apply_system();
            });
            return LRESULT(0);
        }
        WM_CLOSE => {
            with_dock(|d| d.shutdown());
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }
        WM_ENDSESSION => {
            // Windows is signing out or shutting down: put the taskbar back first.
            if wp.0 != 0 {
                with_dock(|d| d.shutdown());
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            with_dock(|d| d.shutdown());
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {}
    }
    let sh = SHELLHOOK.load(std::sync::atomic::Ordering::Relaxed);
    if sh != 0 && msg == sh {
        if wp.0 == HSHELL_FLASH {
            let h = HWND(lp.0 as _);
            with_dock(|d| d.on_flash(h));
        }
        return LRESULT(0);
    }
    let tc = TASKBAR_CREATED.load(std::sync::atomic::Ordering::Relaxed);
    if tc != 0 && msg == tc {
        // Explorer restarted: its new taskbar needs hiding and our AppBar re-registering.
        with_dock(|d| {
            if d.taskbar_hidden { taskbar::set_visible(false); }
            d.appbar_edge = None;
            d.appbar_rect = None;
            d.apply_system();
        });
        crate::tray::on_taskbar_created();
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Put text on the clipboard (owned by `owner`, as Windows requires for SetClipboardData).
fn set_clipboard_text(owner: HWND, text: &str) {
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::Foundation::GlobalFree;
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    const CF_UNICODETEXT: u32 = 13;
    let w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(Some(owner)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(g) as *mut u16;
            let mut given = false;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(g);
                // On success the clipboard owns the memory.
                given = SetClipboardData(CF_UNICODETEXT, Some(windows::Win32::Foundation::HANDLE(g.0))).is_ok();
            }
            if !given {
                let _ = GlobalFree(Some(g));
            }
        }
        let _ = CloseClipboard();
    }
}

fn history_path() -> std::path::PathBuf {
    config::dir().join("search-history.json")
}

fn load_history() -> laminar_sys::history::History {
    std::fs::read_to_string(history_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_history(h: &laminar_sys::history::History) {
    if let Ok(t) = serde_json::to_string(h) {
        let _ = std::fs::write(history_path(), t);
    }
}

/// Text on the clipboard, if any.
fn clipboard_text() -> Option<String> {
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        OpenClipboard(None).ok()?;
        let out = (|| {
            let h = GetClipboardData(CF_UNICODETEXT).ok()?;
            let g = windows::Win32::Foundation::HGLOBAL(h.0);
            // Clipboard contents come from other processes. Do not scan beyond
            // the actual allocation, even if malformed data has no terminator.
            let words = (GlobalSize(g) / std::mem::size_of::<u16>()).min(64 * 1024);
            if words == 0 { return None; }
            let p = GlobalLock(g) as *const u16;
            if p.is_null() { return None; }
            let data = std::slice::from_raw_parts(p, words);
            let text = crate::launchpad_input::clipboard_text(data);
            let _ = GlobalUnlock(g);
            text
        })();
        let _ = CloseClipboard();
        out
    }
}

fn user_name() -> String {
    let n = std::env::var("USERNAME").unwrap_or_else(|_| "User".into());
    let mut c = n.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => n,
    }
}

/// OLE drop target callback: route to the dock.
fn drop_callback(ev: crate::filedrop::Event) -> crate::filedrop::Target {
    let mut t = crate::filedrop::Target::None;
    with_dock(|d| t = d.on_drop_event(ev));
    t
}

/// OLE drop target callback for the window previews.
fn preview_drop_callback(ev: crate::filedrop::Event) -> crate::filedrop::Target {
    with_dock(|d| d.on_preview_drag(ev));
    crate::filedrop::Target::None
}

/// Window procedure shared by all pop-up windows; the dock handles their messages.
extern "system" fn popup_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let mut r = None;
    with_dock(|d| r = d.popup_message(hwnd, msg, wp, lp));
    r.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wp, lp) })
}

/// The display this dock lives on (primary, or the helper's display) and its DPI.
fn this_display(opts: &Options) -> (RECT, u32) {
    let all = crate::displays::all();
    let d = opts.monitor.and_then(|i| all.get(i).copied()).or_else(|| all.first().copied());
    match d {
        Some(d) => (d.rect, d.dpi),
        None => (primary_monitor(), 96),
    }
}

impl Dock {
    /// Is a full-screen app in front of this display?
    ///
    /// Games, video players and presentations cover the whole monitor and must not get a
    /// panel drawn over them. A window counts when it takes the whole monitor and is not
    /// merely maximized: maximized windows stay behind the panel, the way a taskbar does,
    /// while exclusive and borderless full screen gets the display to itself.
    fn fullscreen_in_front(&self) -> bool {
        // Fullscreen geometry belongs to the actual foreground window, not its
        // taskbar representative: a modal or owned popup can cover the display.
        let fg = unsafe { GetForegroundWindow() };
        if fg.0.is_null() || is_own_window(fg) {
            return false;
        }
        unsafe {
            if !IsWindowVisible(fg).as_bool() || IsIconic(fg).as_bool() || IsZoomed(fg).as_bool() {
                return false;
            }
            let mut window = RECT::default();
            if GetWindowRect(fg, &mut window).is_err() {
                return false;
            }
            // The desktop and the taskbar are not apps.
            let mut name = [0u16; 64];
            let length = GetClassNameW(fg, &mut name).max(0) as usize;
            let class = String::from_utf16_lossy(&name[..length.min(name.len())]);
            if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "AuraExplorertop" | "ZSFence") {
                return false;
            }
            if tracker::is_explorer_taskbar(fg) {
                return false;
            }
            // Two pixels of slack: full-screen windows often report one pixel less.
            let monitor = self.monitor;
            let slack = 2;
            window.left <= monitor.left + slack
                && window.top <= monitor.top + slack
                && window.right >= monitor.right - slack
                && window.bottom >= monitor.bottom - slack
        }
    }

    /// Show the panel only while no full-screen app is in front.
    fn update_fullscreen(&mut self) {
        self.fg = tracker::foreground();
        let hidden = self.cfg.hide_fullscreen && self.fullscreen_in_front();
        if hidden == self.hidden_for_fullscreen {
            return;
        }
        self.hidden_for_fullscreen = hidden;
        if hidden {
            trace!("full-screen app in front: hiding the panel");
            if self.launchpad.open {
                self.close_launchpad();
            }
            self.close_control_center();
            self.hide_tray_popup();
            self.mini_tray.hide();
            self.set_tray_hover(None, false);
            self.menu.hide(false);
            self.menu_open = false;
            self.preview.hide();
            self.stack_view.hide();
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        } else {
            trace!("full-screen app gone: showing the panel");
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.refresh(true);
            self.resync_hover();
            self.apply_region();
        }
    }

    /// Start, stop or restart the desktop clock card from the current settings.
    ///
    /// The card is a window of `widgets.exe` — its own process — since 0.3.19: a redraw
    /// problem or a hang on the desktop must not take the panel with it. The panel is still
    /// the single writer of the settings, and it is still the one that draws the card's menu;
    /// the card only reports that it was dragged or right-clicked.
    fn restart_clock(&mut self) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if !self.is_main_dock() {
            return;
        }
        // Whatever is running was started with the previous settings.
        if let Some(mut child) = self.clock_host.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if !self.cfg.desktop_clock {
            return;
        }
        let Ok(panel) = std::env::current_exe() else { return };
        // `widgets.exe` sits next to the panel; without it (an installation updated by
        // copying just the panel) the panel hosts the card in a second process of its own,
        // through the same code.
        let (exe, as_component) = match panel.parent().map(|d| d.join("widgets.exe")).filter(|p| p.is_file()) {
            Some(widgets) => (widgets, false),
            None => (panel, true),
        };
        let mut command = std::process::Command::new(exe);
        if as_component {
            command.arg("--component").arg("clock");
        }
        let child = command
            .arg("--dock")
            .arg(format!("{:#x}", self.hwnd.0 as isize))
            .arg("--parent")
            .arg(std::process::id().to_string())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        match child {
            Ok(child) => {
                self.clock_host = Some(child);
                trace!("desktop clock: hosted by widgets.exe (pid {})", self.clock_host.as_ref().map(|c| c.id()).unwrap_or(0));
            }
            Err(error) => trace!("desktop clock unavailable: {error:?}"),
        }
    }

    /// Start the desktop clock again if its process died.
    ///
    /// A card that fails at once must not turn into a restart loop (the lesson of the
    /// visibility guard): a second attempt waits, and every failed attempt waits again.
    pub(super) fn supervise_clock(&mut self) {
        if !self.cfg.desktop_clock || !self.is_main_dock() {
            return;
        }
        let running = self.clock_host.as_mut().is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        let now = Instant::now();
        if !crate::desktop_retry::due(running, now, self.clock_retry_at) {
            return;
        }
        self.clock_host = None;
        self.clock_retry_at = now + crate::desktop_retry::BACKOFF;
        trace!("desktop clock: the card exited; starting it again");
        self.restart_clock();
    }

    /// Start, stop or restart the desktop load card from the current settings.
    ///
    /// Like the clock, it is a window of `widgets.exe` — a process of its own — so a slow
    /// reading of the processor cannot stall the panel. The panel is still the only writer of
    /// the settings and the only thing that draws the card's menu.
    fn restart_system(&mut self) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if !self.is_main_dock() {
            return;
        }
        if let Some(mut child) = self.system_host.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if !self.cfg.desktop_system {
            return;
        }
        let Ok(panel) = std::env::current_exe() else { return };
        // `widgets.exe` next to the panel, or the panel itself as that same program (an
        // installation updated by copying just the panel).
        let exe = match panel.parent().map(|d| d.join("widgets.exe")).filter(|p| p.is_file()) {
            Some(widgets) => widgets,
            None => panel,
        };
        let mut command = std::process::Command::new(exe);
        // Both pick their card from `--component`; without the flag they draw the clock.
        command.arg("--component").arg("system");
        let child = command
            .arg("--dock")
            .arg(format!("{:#x}", self.hwnd.0 as isize))
            .arg("--parent")
            .arg(std::process::id().to_string())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        match child {
            Ok(child) => {
                self.system_host = Some(child);
                trace!("desktop load card: hosted by widgets.exe (pid {})", self.system_host.as_ref().map(|c| c.id()).unwrap_or(0));
            }
            Err(error) => trace!("desktop load card unavailable: {error:?}"),
        }
    }

    /// Start the desktop load card again if its process died, on the same 30-second backoff
    /// the clock uses: a card that fails at once must not turn into a restart loop.
    pub(super) fn supervise_system(&mut self) {
        if !self.cfg.desktop_system || !self.is_main_dock() {
            return;
        }
        let running = self.system_host.as_mut().is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        let now = Instant::now();
        if !crate::desktop_retry::due(running, now, self.system_retry_at) {
            return;
        }
        self.system_host = None;
        self.system_retry_at = now + crate::desktop_retry::BACKOFF;
        trace!("desktop load card: starting it again");
        self.restart_system();
    }

    /// The load card's menu: where it sits, how big it is, and how to get rid of it.
    fn system_menu(&mut self) {
        if !self.cfg.desktop_system {
            return;
        }
        let prefs = crate::desktop_system::Prefs::from_config(&self.cfg);
        let mut m = vec![MenuItem::header("Карточка нагрузки")];
        for (index, corner) in crate::config::ClockCorner::ALL.iter().enumerate() {
            m.push(MenuItem::item(format!("Угол: {}", corner.label()), CMD_SYSTEM_CORNER_BASE + index).checked(prefs.corner == *corner));
        }
        m.push(MenuItem::sep());
        for (index, (name, scale)) in CLOCK_SIZE_NAMES.iter().zip(CLOCK_SCALES).enumerate() {
            let name = match *name { "Compact" => "Компактный", "Large" => "Крупный", _ => "Обычный" };
            m.push(MenuItem::item(format!("Размер: {name}"), CMD_SYSTEM_SCALE_BASE + index).checked((prefs.scale - scale).abs() < 0.01));
        }
        m.push(MenuItem::sep());
        m.push(MenuItem::item("Закрепить положение", CMD_SYSTEM_LOCK).checked(self.cfg.system_locked));
        m.push(MenuItem::item("Скрыть карточку", CMD_SYSTEM_HIDE).danger());
        let mut point = POINT::default();
        let _ = get_cursor_pos(&mut point);
        self.hide_preview();
        self.menu_open = true;
        if let Err(error) = self.menu.show(&self.compositor, &self.gfx, &self.backdrop, m, point, false, self.scale, self.monitor) {
            trace!("load card menu failed: {error:?}");
            self.menu_open = false;
        }
    }

    /// The clock card's menu: where it sits, how big it is and what it shows.
    fn clock_menu(&mut self) {
        // The card runs in another process: ask the settings, which the panel owns.
        if !self.cfg.desktop_clock {
            return;
        }
        let prefs = crate::desktop_clock::Prefs::from_config(&self.cfg);
        let mut m = vec![MenuItem::header("Desktop Clock")];
        for (index, corner) in crate::config::ClockCorner::ALL.iter().enumerate() {
            m.push(MenuItem::item(format!("Corner: {}", corner.label()), CMD_CLOCK_CORNER_BASE + index).checked(prefs.corner == *corner));
        }
        m.push(MenuItem::sep());
        for (index, (name, scale)) in CLOCK_SIZE_NAMES.iter().zip(CLOCK_SCALES).enumerate() {
            m.push(MenuItem::item(format!("Size: {name}"), CMD_CLOCK_SCALE_BASE + index).checked((prefs.scale - scale).abs() < 0.01));
        }
        m.push(MenuItem::sep());
        m.push(MenuItem::item("24-Hour Time", CMD_CLOCK_24H).checked(!prefs.hour12));
        m.push(MenuItem::item("Show the Date", CMD_CLOCK_DATE).checked(prefs.date));
        m.push(MenuItem::item("Show Seconds", CMD_CLOCK_SECONDS).checked(prefs.seconds));
        m.push(MenuItem::item("Lock Position", CMD_CLOCK_LOCK).checked(self.cfg.clock_locked));
        m.push(MenuItem::sep());
        m.push(MenuItem::item("Hide the Clock", CMD_CLOCK_HIDE).danger());
        let mut point = POINT::default();
        let _ = get_cursor_pos(&mut point);
        self.hide_preview();
        self.menu_open = true;
        if let Err(error) = self.menu.show(&self.compositor, &self.gfx, &self.backdrop, m, point, false, self.scale, self.monitor) {
            trace!("clock menu failed: {error:?}");
            self.menu_open = false;
        }
    }
}

fn primary_monitor() -> RECT {
    unsafe {
        let m = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(m, &mut mi);
        mi.rcMonitor
    }
}

pub fn run(opts: Options, started: Instant) -> Result<()> {
    let integrate = !(opts.measure || opts.demo_hover || opts.demo.is_some());

    // Settings: first run writes defaults with the apps installed on this PC.
    let loaded = Config::load();
    let profile_unreadable = loaded.is_none() && Config::exists();
    let mut cfg = match loaded {
        Some(c) => c,
        None => {
            let mut c = Config::default();
            c.pins = apps::default_pins();
            // Only create the file if there isn't one; never overwrite a file that
            // merely failed to parse (it may hold the user's pins).
            if !Config::exists() {
                if let Err(error) = c.save() { trace!("dock settings were not saved: {error}"); }
            }
            c
        }
    };
    // The control centre's own button: added once, and only if the panel never had it.
    if cfg.ensure_control_center_pin() && Config::exists() {
        if let Err(error) = cfg.save() { trace!("dock settings were not saved: {error}"); }
    }

    unsafe {
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), std::sync::atomic::Ordering::Relaxed);
        let instance = GetModuleHandleW(None)?;
        let class = w!("AuraExplorerDock");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            // No `WS_EX_TOPMOST` up front: the band depends on the edge (see
            // `Dock::place_window`), and a window created topmost would have to be talked
            // back out of it before the first rebuild.
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("AuraDock"),
            WS_POPUP,
            0,
            0,
            100,
            100,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        if profile_unreadable && integrate {
            report_settings_error(hwnd, &std::io::Error::new(std::io::ErrorKind::InvalidData,
                "Не удалось прочитать config.json; восстановите файл перед сохранением настроек."));
        }
        // Allow the live desktop blur, and keep the dock visible during Aero Peek.
        // DWMWA_USE_HOSTBACKDROPBRUSH is Windows 11 only: on Windows 10 it fails and the
        // dock just loses the blur, so never let it stop startup.
        let on: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &on as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_EXCLUDED_FROM_PEEK, &on as *const _ as _, 4);
        // Start fully clipped so nothing is clickable until the dock is built.
        SetWindowRgn(hwnd, Some(CreateRectRgn(0, 0, 0, 0)), false);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let (monitor, dpi) = this_display(&opts);
        let scale = dpi as f32 / 96.0;
        let compositor = Compositor::new()?;
        let target = compositor.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, true)?;
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let gfx = Gfx::new(&compositor)?;
        let ptr = compositor.CreatePropertySet()?;
        ptr.InsertScalar(&hs("PointerX"), 0.0)?;
        ptr.InsertScalar(&hs("Amount"), 0.0)?;
        ptr.InsertScalar(&hs("Delta"), 0.0)?;
        let dock_visual = compositor.CreateContainerVisual()?;
        root.Children()?.InsertAtTop(&dock_visual)?;
        crate::ui::register_popup_class(popup_wndproc)?;
        let menu = Menu::new(&compositor)?;
        let preview = Preview::new(&compositor)?;
        let stack_view = StackView::new(&compositor)?;
        let launchpad = Launchpad::new(&compositor)?;
        let control_center = crate::control_center::ControlCenter::new(&compositor)?;
        let tray_popup = crate::tray_strip::TrayPopup::new(&compositor)?;
        let mini_tray = crate::tray_mini::MiniTray::new(&compositor)?;

        let mut dock = Dock {
            hwnd,
            opts: opts.clone(),
            started,
            geo: Geo::new(&[], &cfg, scale, (monitor.right - monitor.left) as f32, false),
            cfg,
            monitor,
            scale,
            backdrop: test_backdrop(&compositor).map(Ok).unwrap_or_else(|| compositor.CreateHostBackdropBrush()?.cast())?,
            host_backdrop: test_backdrop(&compositor).map(Ok).unwrap_or_else(|| compositor.CreateHostBackdropBrush()?.cast())?,
            label: compositor.CreateContainerVisual()?,
            label_geom: compositor.CreateRoundedRectangleGeometry()?,
            label_text: compositor.CreateSpriteVisual()?,
            dock: dock_visual,
            compositor,
            _target: target,
            gfx,
            icons: IconCache::open(),
            brushes: HashMap::new(),
            tray_brushes: HashMap::new(),
            tray_items: Vec::new(),
            tray_bar_keys: Vec::new(),
            tray_overflow_keys: Vec::new(),
            tray_popup,
            mini_tray,
            mini_press: None,
            tray_hover: None,
            tray_tip_open: None,
            tray_custom_popup: None,
            tray_press: None,
            tray_drag: None,
            tray_highlights: Vec::new(),
            names: HashMap::new(),
            ptr,
            root,
            slots: Vec::new(),
            accs: Vec::new(),
            tiles: Vec::new(),
            inds: Vec::new(),
            ind_state: Vec::new(),
            atts: Vec::new(),
            attention: Default::default(),
            items: Vec::new(),
            tracker: Tracker::default(),
            windows: Vec::new(),
            running_order: Vec::new(),
            fg: HWND::default(),
            hidden_for_fullscreen: false,
            retheme_pending: false,
            resolved: HashMap::new(),
            hovering: false,
            hover: None,
            pressed: None,
            tucked: false,
            menu_open: false,
            menu,
            drag: None,
            press: None,
            ghost: None,
            preview,
            preview_pending: None,
            stack_view,
            menu_stack: None,
            launchpad,
            control_center,
            services: Services::start(hwnd),
            lp_apps: Vec::new(),
            control: ControlState::default(),
            stats: None,
            weather: None,
            files: Vec::new(),
            history: load_history(),
            files_gen: crate::launchpad_input::SearchEpoch::default(),
            icons_requested: std::collections::HashSet::new(),
            user_name: user_name(),
            lp_drag: None,
            lp_press: None,
            lp_menu_app: None,
            drop_target: None,
            file_drag: None,
            wifi: None,
            update: None,
            update_asked: None,
            _drop: None,
            _preview_drop: None,
            drag_under: None,
            drag_switch: None,
            drag_preview: false,
            app_update: None,
            app_update_manual: false,
            menu_ctx: None,
            integrate,
            helper: opts.monitor.is_some(),
            helpers: crate::displays::Helpers::default(),
            appbar_edge: None,
            appbar_rect: None,
            tray_original: None,
            taskbar_hidden: false,
            taskbar_watchdog: None,
            measure_step: 0,
            m_cpu0: 0.0,
            m_t0: Instant::now(),
            report: Vec::new(),
            sweep_t0: Instant::now(),
            sweep_moves: 0,
            last_scan_ms: 0.0,
            last_refresh_ms: 0.0,
            hub: crate::widgets::Hub::new(hwnd),
            wviews: HashMap::new(),
            ptr_xy: (0.0, 0.0),
            anchor_x: 0.0,
            menu_widget: None,
            menu_lpw: None,
            lpw_press: None,
            wtick: 0,
            clock_host: None,
            clock_retry_at: Instant::now(),
            system_host: None,
            system_retry_at: Instant::now(),
            floor_y: 0,
            panel_top: false,
            band_topmost: false,
            z_placed: false,
            start_panes: Vec::new(),
            start_open: false,
        };
        // Until the appbar answers, the panel sits on the monitor's own edge: the bottom
        // when the Windows taskbar is hidden, the top when the taskbar keeps the bottom.
        dock.panel_top = !dock.cfg.hide_windows_taskbar;
        dock.floor_y = if dock.panel_top { dock.monitor.top } else { dock.monitor.bottom };
        dock.update_theme();
        dock.update_glass();
        dock.build_label()?;

        let before_dock_ms = dock.started.elapsed().as_secs_f64() * 1000.0;
        dock.rescan(); // finds running windows and builds the dock
        let _ = DwmFlush();
        let startup_ms = dock.started.elapsed().as_secs_f64() * 1000.0;
        let _hooks = tracker::install_hooks(hwnd);
        // Hear about windows flashing for attention, like the Windows taskbar does.
        SHELLHOOK.store(RegisterWindowMessageW(w!("SHELLHOOK")), std::sync::atomic::Ordering::Relaxed);
        let shell_hook = RegisterShellHookWindow(hwnd).as_bool();
        trace!("shell hook registered: {shell_hook}");
        dock._drop = match crate::filedrop::register(hwnd, drop_callback) {
            Ok(t) => Some(t),
            Err(e) => {
                trace!("file drop unavailable: {e:?}");
                None
            }
        };
        trace!("file drop target registered: {}", dock._drop.is_some());
        // Drags can also rest on window tiles, to bring a window forward and drop into it.
        dock._preview_drop = crate::filedrop::register(dock.preview.hwnd(), preview_drop_callback).ok();
        dock.apply_system();
        // Warm up Launchpad data in the background; refresh the weather every 30 minutes.
        dock.services.send(Req::Apps);
        dock.services.send(Req::Weather(dock.cfg.weather_lat, dock.cfg.weather_lon));
        dock.services.send(Req::Control);
        SetTimer(Some(hwnd), T_WEATHER, 30 * 60 * 1000, None);
        SetTimer(Some(hwnd), T_WIDGETS, 1000, None);
        dock.hub.ask_mic();
        dock.hub.ask_output();
        dock.hub.ask_brightness();
        // Banners off is Windows' own switch: start the tile where Windows is.
        dock.hub.dnd = laminar_sys::night::notifications_enabled().map(|on| !on).unwrap_or(false);
        let main_dock = integrate && opts.monitor.is_none();
        dock.restart_clock();
        let _kb = if main_dock { crate::winkey::install(hwnd, dock.cfg.win_key_launchpad) } else { None };
        if main_dock {
            // Host the tray icons apps put beside the clock (shown in Launchpad).
            crate::tray::start(hwnd, WM_APP_TRAY);
            // Retry if Explorer has not created its taskbar yet.
            SetTimer(Some(hwnd), T_TRAY_RAISE, 1500, None);
            // Installed copies: match the sign-in switch to the Startup shortcut (never
            // created here, only from Settings), and look for updates once things settle.
            match crate::startup::current() {
                Ok(actual) if actual != dock.cfg.start_at_login => {
                    dock.cfg.start_at_login = actual;
                    if let Err(error) = dock.cfg.save() { trace!("dock settings were not saved: {error}"); }
                }
                Err(error) => trace!("could not read Dock startup entry: {error}"),
                _ => {}
            }
            if crate::updater::installed() { SetTimer(Some(hwnd), T_UPDATE_CHECK, 20_000, None); }
            dock.refresh(true);
            trace!("AuraDock {}", crate::updater::version_label());
        }
        if let Some(pid) = opts.parent {
            crate::displays::watch_parent(pid, hwnd);
        }

        if dock.opts.measure {
            let m = metrics::memory();
            dock.report.push(("startup_ms".into(), format!("{startup_ms:.0}")));
            dock.report.push(("startup_before_dock_ms".into(), format!("{before_dock_ms:.0}")));
            dock.report.push(("startup_window_scan_ms".into(), format!("{:.0}", dock.last_scan_ms)));
            dock.report.push(("startup_icons_and_build_ms".into(), format!("{:.0}", dock.last_refresh_ms)));
            dock.report.push(("icons".into(), format!("{}", dock.items.iter().filter(|i| i.is_icon()).count())));
            dock.report.push(("running_windows".into(), format!("{}", dock.windows.len())));
            dock.report.push(("private_bytes_at_start_mb".into(), format!("{:.1}", m.private_bytes)));
            SetTimer(Some(hwnd), T_MEASURE, 2_000, None);
        }
        if dock.opts.demo_hover {
            let g = &dock.geo;
            let (x, y) = (g.center_abs(4.min(g.left.len().saturating_sub(1))), g.y0 + g.shelf_h / 2.0);
            dock.on_move(x, y);
        }
        if let Some(name) = dock.opts.demo.clone() {
            dock.run_demo(&name);
        }
        // `AuraDock.exe --settings`: the panel opens on its own settings, like the «Настройки»
        // button does — the same page, the same code, only the way in differs. This runs before
        // the dock is published to `DOCK`, so it goes through `dock` itself.
        if dock.opts.settings {
            dock.open_launchpad();
            dock.lp_act(Act::OpenSettings);
        }
        if let Some(secs) = dock.opts.exit_after {
            SetTimer(Some(hwnd), T_EXIT, secs * 1000, None);
        }
        if dock.cfg.auto_hide && integrate && !dock.opts.settings {
            // Asked for its settings from another program (the AuraUI settings window), the
            // panel must stay on screen: tucking away after a second and a half took the
            // settings page with it, which read as "the dock's settings never open".
            SetTimer(Some(hwnd), T_HIDE, 1500, None); // show briefly at start, then tuck away
        }
        DOCK.with(|d| *d.borrow_mut() = Some(dock));

        // Message loop that also wakes when the settings folder changes (no polling).
        let _ = std::fs::create_dir_all(config::dir());
        let watch: Option<HANDLE> =
            FindFirstChangeNotificationW(&HSTRING::from(config::dir().as_os_str()), false, FILE_NOTIFY_CHANGE_LAST_WRITE).ok();
        let handles: Vec<HANDLE> = watch.into_iter().collect();
        let mut msg = MSG::default();
        'outer: loop {
            let r = MsgWaitForMultipleObjects(Some(&handles), false, INFINITE, QS_ALLINPUT);
            if !handles.is_empty() && r == WAIT_OBJECT_0 {
                let _ = FindNextChangeNotification(handles[0]);
                let _ = PostMessageW(Some(hwnd), WM_APP_CONFIG, WPARAM(0), LPARAM(0));
            }
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'outer;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        DOCK.with(|d| *d.borrow_mut() = None);
    }
    Ok(())
}

// Widgets (after the trace! macro, which the module uses).
#[path = "dock_widgets.rs"]
mod dock_widgets;

#[path = "dock_control.rs"]
mod dock_control;
