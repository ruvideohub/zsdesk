//! The dock window itself.
//!
//! A topmost, per-pixel-alpha layered strip along the bottom edge of the
//! monitor. Fully transparent pixels pass mouse input through, so the window
//! may span the whole screen width while only the rounded panel reacts.
//!
//! Painting reuses the Direct2D software target of `zsdesk_windows::gfx` and
//! presents it with `UpdateLayeredWindow`, exactly like the desktop fences.

use std::{
    cell::RefCell,
    path::PathBuf,
    time::{Duration, Instant},
};

use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, SYSTEMTIME, WPARAM},
    Graphics::Gdi::{
        ClientToScreen, GetDC, GetMonitorInfoW, MonitorFromPoint, ReleaseDC, ScreenToClient,
        AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    },
    System::SystemInformation::GetLocalTime,
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, DefWindowProcW, DestroyMenu, DestroyWindow, GetCursorPos,
            KillTimer, MessageBoxW, PostQuitMessage, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow,
            TrackPopupMenu, UpdateLayeredWindow, HMENU, MB_ICONWARNING, MB_OK, MF_CHECKED, MF_GRAYED,
            MF_SEPARATOR, MF_STRING, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOWNOACTIVATE, TPM_LEFTALIGN,
            TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN, ULW_ALPHA, WM_APP, WM_CLOSE, WM_DESTROY,
            WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP,
            WM_SETTINGCHANGE, WM_TIMER,
        },
    },
};

use zsdesk_windows::gfx::{Align, Canvas, Color, Gfx, RectF, Surface, TextStyle};

use crate::{
    apps,
    config::{DockConfig, Pin},
    icons::{IconCache, IconLoader},
    layout::{self, Magnify, Metrics, Rect},
    model::{self, Kind, Slot},
    panel_window, status, tray, winlist,
};

/// Window class of the dock strip.
pub const CLASS: &str = "ZSDesk.Dock";
/// Posted by the icon worker after a batch of bitmaps is ready.
pub const WM_ICON_READY: u32 = WM_APP + 501;
const TIMER_TICK: usize = 1;
/// How often the running-window list and the clock are refreshed.
const REFRESH: Duration = Duration::from_millis(1000);
/// Height of the strip that stays on screen while the dock is hidden.
const SLIVER: f32 = 4.0;
/// Registry value that starts the dock with the user session.
const AUTOSTART_VALUE: &str = "ZSDesk.Dock";

// Menu command ids.
const CMD_WINDOW_BASE: u32 = 10;
const CMD_LAUNCH: u32 = 40;
const CMD_MINIMIZE_ALL: u32 = 41;
const CMD_CLOSE_ALL: u32 = 42;
const CMD_PIN: u32 = 43;
const CMD_UNPIN: u32 = 44;
const CMD_REVEAL: u32 = 45;
const CMD_LAUNCHER: u32 = 60;
const CMD_MAGNIFY: u32 = 61;
const CMD_AUTOHIDE: u32 = 62;
const CMD_CLOCK: u32 = 63;
const CMD_TRASH: u32 = 64;
const CMD_ICON_SMALL: u32 = 65;
const CMD_ICON_MEDIUM: u32 = 66;
const CMD_ICON_LARGE: u32 = 67;
const CMD_AUTOSTART: u32 = 68;
const CMD_PROFILE: u32 = 69;
const CMD_EMPTY_TRASH: u32 = 70;
const CMD_EXIT: u32 = 71;
const CMD_STATUS: u32 = 72;
const CMD_QUICK_SETTINGS: u32 = 73;
const CMD_NOTIFICATIONS: u32 = 74;

thread_local! {
    /// The single dock of this thread. `try_borrow_mut` in the window
    /// procedure keeps a modal menu loop from re-entering the state.
    static DOCK: RefCell<Option<Dock>> = const { RefCell::new(None) };
}

/// Colors of the dock, following the system light/dark preference.
struct Palette {
    panel: Color,
    border: Color,
    text: Color,
    secondary: Color,
    pill: Color,
    accent: Color,
    dot: Color,
    shadow: Color,
}

impl Palette {
    fn for_mode(dark: bool, accent: [u8; 3]) -> Self {
        if dark {
            Self {
                panel: Color::rgba8(30, 30, 34, 198),
                border: Color::rgba8(255, 255, 255, 42),
                text: Color::rgba8(244, 244, 246, 255),
                secondary: Color::rgba8(244, 244, 246, 160),
                pill: Color::rgba8(48, 48, 54, 238),
                accent: Color::rgba8(accent[0], accent[1], accent[2], 255),
                dot: Color::rgba8(244, 244, 246, 220),
                shadow: Color::rgba8(0, 0, 0, 70),
            }
        } else {
            Self {
                panel: Color::rgba8(248, 248, 250, 208),
                border: Color::rgba8(0, 0, 0, 28),
                text: Color::rgba8(24, 24, 28, 255),
                secondary: Color::rgba8(24, 24, 28, 150),
                pill: Color::rgba8(255, 255, 255, 242),
                accent: Color::rgba8(accent[0], accent[1], accent[2], 255),
                dot: Color::rgba8(24, 24, 28, 190),
                shadow: Color::rgba8(0, 0, 0, 44),
            }
        }
    }
}

/// Sizes of the whole icon row, scaled to the window DPI.
struct Geometry {
    metrics: Metrics,
    magnify: Option<Magnify>,
    label_size: f32,
    clock_size: f32,
    dot: f32,
    /// Edge of one status indicator.
    status_item: f32,
    /// Space between status indicators.
    status_gap: f32,
}

impl Geometry {
    fn new(config: &DockConfig, dpi: u32) -> Self {
        let factor = (dpi.max(96) as f32) / 96.0;
        let icon = config.icon_size;
        Self {
            metrics: Metrics { icon, gap: 8.0, padding: 10.0, panel: icon + 18.0 }.at_scale(factor),
            magnify: config.magnification.then(|| Magnify {
                max: config.magnify_scale,
                influence: config.magnify_influence,
            }),
            label_size: 12.0 * factor,
            clock_size: 13.0 * factor,
            dot: 4.0 * factor,
            status_item: status::ITEM * factor,
            status_gap: status::GAP * factor,
        }
    }

    /// Indicators the status sector currently shows.
    fn status_kinds(&self, config: &DockConfig) -> Vec<status::Kind> {
        if config.show_status { status::Kind::ALL.to_vec() } else { Vec::new() }
    }

    /// Space the panel keeps on its right edge for the status sector.
    fn status_reserve(&self, config: &DockConfig) -> f32 {
        status::reserve(&self.status_kinds(config), self.status_item, self.status_gap)
    }

    /// Height of the window: magnified icons, the label pill and the panel.
    fn window_height(&self, config: &DockConfig) -> i32 {
        let headroom = if config.magnification { self.metrics.headroom(config.magnify_scale) } else { 0.0 };
        (self.metrics.panel + headroom + self.label_size + 16.0 + SLIVER).ceil().max(48.0) as i32
    }
}

/// A click that is still waiting for its button release.
#[derive(Clone, Copy, Debug)]
struct Press {
    index: usize,
    point: POINT,
}

/// Everything one dock window owns.
pub struct Dock {
    hwnd: HWND,
    gfx: Gfx,
    surface: Option<Surface>,
    config: DockConfig,
    config_path: PathBuf,
    icons: IconCache,
    loader: IconLoader,
    catalogue: Option<Vec<apps::AppEntry>>,
    slots: Vec<Slot>,
    placement: layout::Placement,
    geometry: Geometry,
    area: RECT,
    pointer: Option<POINT>,
    hover: Option<usize>,
    press: Option<Press>,
    /// Indicator under the pointer, if the pointer is not over an icon.
    hover_status: Option<status::Kind>,
    press_status: Option<(status::Kind, POINT)>,
    /// Last sample of network, battery and volume.
    status: tray::Snapshot,
    /// 1.0 = dock shown, 0.0 = only the sliver is visible.
    reveal: f32,
    reveal_target: f32,
    dpi: u32,
    clock: String,
    dark: bool,
    accent: [u8; 3],
    last_refresh: Instant,
    autostart: bool,
    dirty: bool,
}

/// Registers the dock for this thread and shows the first frame.
pub fn create(config_path: PathBuf) -> windows::core::Result<()> {
    let area = cursor_work_area();
    let config = DockConfig::load(&config_path);
    let dock = Dock::new(config, config_path, area)?;
    DOCK.with(|slot| { *slot.borrow_mut() = Some(dock); });
    DOCK.with(|slot| {
        if let Some(dock) = slot.borrow_mut().as_mut() {
            dock.relayout();
            dock.refresh(true);
            let _ = unsafe { ShowWindow(dock.hwnd, SW_SHOWNOACTIVATE) };
            dock.paint();
            unsafe { let _ = SetTimer(Some(dock.hwnd), TIMER_TICK, 16, None); }
        }
    });
    Ok(())
}

/// Destroys the window and stops the icon worker.
pub fn destroy() {
    DOCK.with(|slot| { if let Some(mut dock) = slot.borrow_mut().take() { dock.loader.shutdown(); } });
}

/// Window procedure, registered by `app::run`.
pub unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = DOCK.with(|slot| match slot.try_borrow_mut() {
        Ok(mut slot) => slot.as_mut().and_then(|dock| dock.on_message(hwnd, message, wparam, lparam)),
        // A modal menu loop is running inside the handler below.
        Err(_) => None,
    });
    match handled {
        Some(result) => result,
        None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

impl Dock {
    fn new(config: DockConfig, config_path: PathBuf, area: RECT) -> windows::core::Result<Self> {
        let dark = zsdesk_windows::theme::system_dark_mode();
        let accent = zsdesk_windows::theme::system_accent(dark).unwrap_or([10, 132, 255]);
        let family = zsdesk_windows::gfx::system_ui_font(None).0;
        let gfx = Gfx::new(&family).map_err(|error| {
            windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), error.message())
        })?;
        let width = (area.right - area.left).max(200);
        let geometry = Geometry::new(&config, 96);
        let height = geometry.window_height(&config);

        let class: Vec<u16> = CLASS.encode_utf16().chain(Some(0)).collect();
        let title = zsdesk_core::i18n::wide("码头");
        let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .map(|module| windows::Win32::Foundation::HINSTANCE(module.0))?;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_POPUP};
        let hwnd = unsafe {
            CreateWindowExW(WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()), WS_POPUP, area.left, area.bottom - height, width, height,
                None, None, Some(instance), None)
        }?;
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let geometry = Geometry::new(&config, dpi);
        let height = geometry.window_height(&config);
        let autostart = dock_autostart_enabled();
        Ok(Self {
            hwnd,
            gfx,
            surface: None,
            config,
            config_path,
            icons: IconCache::default(),
            loader: IconLoader::spawn(hwnd, WM_ICON_READY),
            catalogue: None,
            slots: Vec::new(),
            placement: layout::place(0, Rect::new(0.0, 0.0, width as f32, height as f32), geometry.metrics, None, None, 0.0),
            geometry,
            area,
            pointer: None,
            hover: None,
            press: None,
            hover_status: None,
            press_status: None,
            status: tray::Snapshot::default(),
            reveal: 1.0,
            reveal_target: 1.0,
            dpi,
            clock: String::new(),
            dark,
            accent,
            last_refresh: Instant::now() - REFRESH,
            autostart,
            dirty: true,
        })
    }

    fn on_message(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match message {
            WM_TIMER if wparam.0 == TIMER_TICK => { self.on_tick(); Some(LRESULT(0)) }
            WM_ICON_READY => { self.loader.drain(&self.gfx, &mut self.icons); self.dirty = true; Some(LRESULT(0)) }
            WM_MOUSEMOVE => {
                self.set_pointer(Some(point_from_lparam(lparam)));
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let point = point_from_lparam(lparam);
                self.set_pointer(Some(point));
                self.press = self.hover.map(|index| Press { index, point });
                self.press_status = if self.press.is_none() { self.hover_status.map(|kind| (kind, point)) } else { None };
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let released = point_from_lparam(lparam);
                let press = self.press.take();
                let press_status = self.press_status.take();
                self.set_pointer(Some(released));
                if let Some(press) = press {
                    // A drag is not a click; the dock only reacts to taps.
                    let moved = (press.point.x - released.x).abs() > 6 || (press.point.y - released.y).abs() > 6;
                    if !moved && self.hover == Some(press.index) { self.activate(press.index); }
                }
                if let Some((kind, start)) = press_status {
                    let moved = (start.x - released.x).abs() > 6 || (start.y - released.y).abs() > 6;
                    if !moved && self.hover_status == Some(kind) { self.status_click(kind); }
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                let point = point_from_lparam(lparam);
                self.set_pointer(Some(point));
                if let Some(screen) = self.client_to_screen(point) {
                    match self.hover_status {
                        Some(kind) => self.show_status_menu(kind, screen),
                        None => self.show_menu(self.hover, screen),
                    }
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                self.dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
                self.geometry = Geometry::new(&self.config, self.dpi);
                self.area = cursor_work_area();
                self.relayout();
                self.reposition();
                self.dirty = true;
                Some(LRESULT(0))
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                self.area = cursor_work_area();
                self.relayout();
                self.reposition();
                self.dirty = true;
                Some(LRESULT(0))
            }
            WM_CLOSE => { let _ = unsafe { DestroyWindow(hwnd) }; Some(LRESULT(0)) }
            WM_DESTROY => {
                let _ = unsafe { KillTimer(Some(hwnd), TIMER_TICK) };
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    /// 16 ms heartbeat: magnification animation, autohide and the once-per-
    /// second refresh of the window list, the clock and the icon queue.
    fn on_tick(&mut self) {
        let previous = self.reveal;
        self.reveal = crate::magnify::approach(self.reveal, self.reveal_target, 0.3);
        if (previous - self.reveal).abs() > 0.001 { self.dirty = true; }
        if self.last_refresh.elapsed() >= REFRESH {
            self.last_refresh = Instant::now();
            self.update_pointer_from_system();
            self.clock = clock_text(self.config.clock_24h);
            self.refresh(false);
            self.dark = zsdesk_windows::theme::system_dark_mode();
            self.accent = zsdesk_windows::theme::system_accent(self.dark).unwrap_or([10, 132, 255]);
            self.dirty = true;
        }
        self.loader.drain(&self.gfx, &mut self.icons);
        if self.dirty {
            self.dirty = false;
            self.paint();
        }
    }

    fn update_pointer_from_system(&mut self) {
        let mut cursor = POINT::default();
        if unsafe { GetCursorPos(&mut cursor) }.is_err() { self.set_pointer(None); return; }
        let mut client = cursor;
        if unsafe { ScreenToClient(self.hwnd, &mut client) }.as_bool() {
            self.set_pointer(Some(client));
        } else {
            self.set_pointer(None);
        }
    }

    /// Stores the pointer, recomputes hover and the autohide target.
    fn set_pointer(&mut self, point: Option<POINT>) {
        let previous_hover = self.hover;
        let previous_target = self.reveal_target;
        let previous_status = self.hover_status;
        self.pointer = point;
        let offset = layout::hidden_offset(self.reveal, self.geometry.metrics.panel, SLIVER);
        self.hover = point.and_then(|point| {
            let x = point.x as f32;
            let y = point.y as f32 - offset;
            self.placement.hit(x, y)
        });
        let items = self.status_items();
        self.hover_status = point.and_then(|point| {
            if self.hover.is_some() { return None }
            status::hit(&items, point.x as f32, point.y as f32 - offset)
        });
        if self.config.autohide {
            let inside = point.is_some_and(|point| {
                let panel = self.placement.panel;
                let x = point.x as f32;
                let y = point.y as f32;
                // The whole panel plus the sliver at the bottom edge reveals.
                (x >= panel.x - 24.0 && x <= panel.right() + 24.0 && y >= panel.y - offset && y <= self.window_height())
                    || y >= self.window_height() - SLIVER - 2.0
            });
            self.reveal_target = if inside { 1.0 } else { 0.0 };
        } else {
            self.reveal_target = 1.0;
        }
        if previous_hover != self.hover || previous_status != self.hover_status
            || (previous_target - self.reveal_target).abs() > 0.001 {
            self.dirty = true;
        }
    }

    fn window_height(&self) -> f32 { self.geometry.window_height(&self.config) as f32 }

    /// Rebuilds the icon row and the slot geometry.
    fn relayout(&mut self) {
        let area = Rect::new(0.0, 0.0, (self.area.right - self.area.left) as f32, self.window_height());
        let pointer = self.pointer.map(|point| point.x as f32);
        let reserve = self.geometry.status_reserve(&self.config);
        self.placement = layout::place(self.slots.len(), area, self.geometry.metrics, pointer,
            self.geometry.magnify, reserve);
    }

    /// Places the window on the monitor and keeps its height in step with the
    /// current DPI and icon size.
    fn reposition(&mut self) {
        let width = (self.area.right - self.area.left).max(200);
        let height = self.geometry.window_height(&self.config);
        let _ = unsafe { SetWindowPos(self.hwnd, None, self.area.left, self.area.bottom - height, width, height,
            SWP_NOZORDER | SWP_NOACTIVATE) };
    }

    /// Re-reads the running applications; `force` also rebuilds the first time.
    fn refresh(&mut self, force: bool) {
        let sample = tray::query();
        if sample != self.status { self.status = sample; self.dirty = true; }
        let windows = winlist::snapshot();
        let groups = model::group_by_exe(windows);
        let slots = model::build_slots(&self.config.pins, groups, self.config.show_launcher, self.config.show_trash);
        if force || slots != self.slots {
            self.slots = slots;
            self.relayout();
            self.request_icons();
            self.dirty = true;
        }
    }

    /// Queues icon loads for slots whose bitmap is still missing.
    fn request_icons(&mut self) {
        let pixels = (self.geometry.metrics.icon * self.config.magnify_scale.clamp(1.0, 3.0)).round() as i32;
        let mut requests = Vec::new();
        for slot in &self.slots {
            if slot.kind != Kind::App { continue; }
            if self.icons.want(&slot.key) { requests.push((slot.key.clone(), slot.target.clone())); }
        }
        for (key, target) in requests { self.loader.request(&key, &target, pixels.clamp(32, 256)); }
    }


    /// Indicator rectangles of the current panel, without the slide offset.
    fn status_items(&self) -> Vec<(status::Kind, Rect)> {
        status::layout(self.placement.panel, &self.geometry.status_kinds(&self.config),
            self.geometry.status_item, self.geometry.status_gap, status::CLOCK)
    }

    /// Left click on an indicator. The dock does not own the notification area,
    /// so every indicator opens the Windows surface that does.
    fn status_click(&mut self, kind: status::Kind) {
        match kind {
            // The card shows live volume and the quick actions in one place.
            status::Kind::Chevron | status::Kind::Volume => self.open_panel(kind),
            status::Kind::Network => { let _ = apps::launch("ms-availablenetworks:"); }
            status::Kind::Battery => { let _ = apps::launch("ms-settings:batterysaver-settings"); }
        }
    }

    /// Opens the sound and status card above `kind`; a second click closes it.
    fn open_panel(&mut self, kind: status::Kind) {
        if panel_window::is_open() {
            panel_window::close();
            return;
        }
        let Some((_, rect)) = self.status_items().into_iter().find(|(item, _)| *item == kind) else { return };
        let origin = POINT { x: rect.x as i32, y: rect.y as i32 };
        let Some(screen) = self.client_to_screen(origin) else { return };
        let anchor = RECT { left: screen.x, top: screen.y, right: rect.w as i32, bottom: rect.h as i32 };
        if let Err(error) = panel_window::open(anchor, self.area, self.dark, self.accent) {
            self.show_error(&error.to_string());
        }
    }

    /// Right click on an indicator.
    fn show_status_menu(&mut self, kind: status::Kind, screen: POINT) {
        let Ok(menu) = (unsafe { CreatePopupMenu() }) else { return };
        append(&menu, zsdesk_core::i18n::text(kind.label()), MF_STRING | MF_GRAYED, 0);
        append(&menu, "", MF_SEPARATOR, 0);
        append(&menu, zsdesk_core::i18n::text("快速设置"), MF_STRING, CMD_QUICK_SETTINGS);
        append(&menu, zsdesk_core::i18n::text("通知中心"), MF_STRING, CMD_NOTIFICATIONS);
        append(&menu, "", MF_SEPARATOR, 0);
        let status_flag = MF_STRING | if self.config.show_status { MF_CHECKED } else { Default::default() };
        append(&menu, zsdesk_core::i18n::text("显示状态指示器"), status_flag, CMD_STATUS);
        let commands = vec![
            (CMD_QUICK_SETTINGS, MenuAction::QuickSettings),
            (CMD_NOTIFICATIONS, MenuAction::Notifications),
            (CMD_STATUS, MenuAction::SetStatus(!self.config.show_status)),
        ];
        self.track(menu, screen, commands);
    }

    /// Shows a menu and runs the chosen command once the menu is gone.
    fn track(&mut self, menu: HMENU, screen: POINT, commands: Vec<(u32, MenuAction)>) {
        let _ = unsafe { SetForegroundWindow(self.hwnd) };
        let chosen = unsafe {
            TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN,
                screen.x, screen.y, None, self.hwnd, None)
        };
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::PostMessageW(Some(self.hwnd),
            windows::Win32::UI::WindowsAndMessaging::WM_NULL, WPARAM(0), LPARAM(0)) };
        let _ = unsafe { DestroyMenu(menu) };
        let id = u32::try_from(chosen.0).unwrap_or(0);
        if let Some((_, action)) = commands.into_iter().find(|(command, _)| *command == id) {
            self.run(action);
        }
    }

    /// Left click on an icon.
    fn activate(&mut self, index: usize) {
        let Some(slot) = self.slots.get(index).cloned() else { return };
        match slot.kind {
            Kind::App => {
                if let Some(window) = winlist::forwardmost(&slot.windows) {
                    // Clicking the frontmost window of a running app hides it,
                    // like the macOS dock does.
                    winlist::minimize(window);
                } else if let Some(window) = slot.windows.first() {
                    winlist::activate(window.hwnd);
                } else if let Err(error) = apps::launch(&slot.target) {
                    self.show_error(&error);
                }
            }
            Kind::Launcher => self.open_launcher(),
            Kind::Trash => apps::open_recycle_bin(),
        }
        self.last_refresh = Instant::now() - REFRESH;
        self.dirty = true;
    }

    fn open_launcher(&mut self) {
        if self.catalogue.is_none() {
            self.catalogue = Some(apps::scan(&apps::start_menu_directories()));
        }
        let entries = self.catalogue.clone().unwrap_or_default();
        match crate::launcher::open(self.area, entries, self.geometry.metrics.icon, self.dark, self.accent) {
            Ok(launcher) => crate::launcher::install(launcher),
            Err(error) => self.show_error(&format!("{}", error.message())),
        }
    }

    fn show_error(&self, message: &str) {
        let body: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let caption = zsdesk_core::i18n::wide("码头");
        unsafe { MessageBoxW(Some(self.hwnd), PCWSTR(body.as_ptr()), PCWSTR(caption.as_ptr()), MB_OK | MB_ICONWARNING) };
    }

    fn client_to_screen(&self, point: POINT) -> Option<POINT> {
        let mut screen = point;
        unsafe { ClientToScreen(self.hwnd, &mut screen) }
            .as_bool()
            .then_some(screen)
    }

    fn save(&mut self) {
        if let Err(error) = self.config.save(&self.config_path) {
            self.show_error(&format!("{}: {error}", zsdesk_core::i18n::text("码头设置")));
        }
    }

    /// Right-click menu for one icon or for the dock background.
    fn show_menu(&mut self, index: Option<usize>, screen: POINT) {
        let Ok(menu) = (unsafe { CreatePopupMenu() }) else { return };
        let mut commands: Vec<(u32, MenuAction)> = Vec::new();
        match index.and_then(|index| self.slots.get(index).cloned().map(|slot| (index, slot))) {
            Some((index, slot)) => {
                let header = if slot.label.is_empty() { zsdesk_core::i18n::text("码头") } else { &slot.label }.to_string();
                append(&menu, &header, MF_STRING | MF_GRAYED, 0);
                append(&menu, "", MF_SEPARATOR, 0);
                for (position, window) in slot.windows.iter().enumerate().take(9) {
                    let title = elide(&window.title, 42);
                    let label = format!("{}. {}", position + 1, title);
                    let id = CMD_WINDOW_BASE + position as u32;
                    append(&menu, &label, MF_STRING, id);
                    commands.push((id, MenuAction::Activate(window.hwnd)));
                }
                if !slot.windows.is_empty() { append(&menu, "", MF_SEPARATOR, 0); }
                match slot.kind {
                    Kind::App => {
                        if !slot.windows.is_empty() {
                            append(&menu, zsdesk_core::i18n::text("关闭全部窗口"), MF_STRING, CMD_CLOSE_ALL);
                            commands.push((CMD_CLOSE_ALL, MenuAction::CloseAll(index)));
                            append(&menu, zsdesk_core::i18n::text("最小化"), MF_STRING, CMD_MINIMIZE_ALL);
                            commands.push((CMD_MINIMIZE_ALL, MenuAction::MinimizeAll(index)));
                        }
                        if slot.pinned {
                            append(&menu, zsdesk_core::i18n::text("新建窗口"), MF_STRING, CMD_LAUNCH);
                            commands.push((CMD_LAUNCH, MenuAction::Launch(index)));
                            append(&menu, zsdesk_core::i18n::text("从码头移除"), MF_STRING, CMD_UNPIN);
                            commands.push((CMD_UNPIN, MenuAction::Unpin(index)));
                        } else {
                            append(&menu, zsdesk_core::i18n::text("在码头固定"), MF_STRING, CMD_PIN);
                            commands.push((CMD_PIN, MenuAction::Pin(index)));
                        }
                        append(&menu, zsdesk_core::i18n::text("显示文件位置"), MF_STRING, CMD_REVEAL);
                        commands.push((CMD_REVEAL, MenuAction::Reveal(index)));
                    }
                    Kind::Trash => {
                        append(&menu, zsdesk_core::i18n::text("清空回收站"), MF_STRING, CMD_EMPTY_TRASH);
                        commands.push((CMD_EMPTY_TRASH, MenuAction::EmptyTrash));
                    }
                    Kind::Launcher => {
                        append(&menu, zsdesk_core::i18n::text("全部应用"), MF_STRING, CMD_LAUNCHER);
                        commands.push((CMD_LAUNCHER, MenuAction::Launcher));
                    }
                }
            }
            None => {
                append(&menu, zsdesk_core::i18n::text("全部应用"), MF_STRING, CMD_LAUNCHER);
                commands.push((CMD_LAUNCHER, MenuAction::Launcher));
                append(&menu, "", MF_SEPARATOR, 0);
                let magnification = MF_STRING | if self.config.magnification { MF_CHECKED } else { Default::default() };
                append(&menu, zsdesk_core::i18n::text("放大图标"), magnification, CMD_MAGNIFY);
                commands.push((CMD_MAGNIFY, MenuAction::SetMagnification(!self.config.magnification)));
                let autohide = MF_STRING | if self.config.autohide { MF_CHECKED } else { Default::default() };
                append(&menu, zsdesk_core::i18n::text("自动隐藏"), autohide, CMD_AUTOHIDE);
                commands.push((CMD_AUTOHIDE, MenuAction::SetAutohide(!self.config.autohide)));
                let clock = MF_STRING | if self.config.show_clock { MF_CHECKED } else { Default::default() };
                append(&menu, zsdesk_core::i18n::text("显示时钟"), clock, CMD_CLOCK);
                commands.push((CMD_CLOCK, MenuAction::SetClock(!self.config.show_clock)));
                let status_flag = MF_STRING | if self.config.show_status { MF_CHECKED } else { Default::default() };
                append(&menu, zsdesk_core::i18n::text("显示状态指示器"), status_flag, CMD_STATUS);
                commands.push((CMD_STATUS, MenuAction::SetStatus(!self.config.show_status)));
                let trash = MF_STRING | if self.config.show_trash { MF_CHECKED } else { Default::default() };
                append(&menu, zsdesk_core::i18n::text("显示回收站"), trash, CMD_TRASH);
                commands.push((CMD_TRASH, MenuAction::SetTrash(!self.config.show_trash)));
                append(&menu, "", MF_SEPARATOR, 0);
                append(&menu, zsdesk_core::i18n::text("小图标"), MF_STRING, CMD_ICON_SMALL);
                commands.push((CMD_ICON_SMALL, MenuAction::IconSize(32.0)));
                append(&menu, zsdesk_core::i18n::text("中图标"), MF_STRING, CMD_ICON_MEDIUM);
                commands.push((CMD_ICON_MEDIUM, MenuAction::IconSize(48.0)));
                append(&menu, zsdesk_core::i18n::text("大图标"), MF_STRING, CMD_ICON_LARGE);
                commands.push((CMD_ICON_LARGE, MenuAction::IconSize(64.0)));
                append(&menu, "", MF_SEPARATOR, 0);
                let autostart = MF_STRING | if self.autostart { MF_CHECKED } else { Default::default() };
                append(&menu, zsdesk_core::i18n::text("启动时运行码头"), autostart, CMD_AUTOSTART);
                commands.push((CMD_AUTOSTART, MenuAction::SetAutostart(!self.autostart)));
                append(&menu, zsdesk_core::i18n::text("打开配置目录"), MF_STRING, CMD_PROFILE);
                commands.push((CMD_PROFILE, MenuAction::OpenProfile));
                append(&menu, "", MF_SEPARATOR, 0);
                append(&menu, zsdesk_core::i18n::text("退出码头"), MF_STRING, CMD_EXIT);
                commands.push((CMD_EXIT, MenuAction::Exit));
            }
        }
        self.track(menu, screen, commands);
    }

    fn run(&mut self, action: MenuAction) {
        match action {
            MenuAction::Activate(hwnd) => winlist::activate(hwnd),
            MenuAction::MinimizeAll(index) => {
                for window in self.slots.get(index).map(|slot| slot.windows.clone()).unwrap_or_default() {
                    winlist::minimize(window.hwnd);
                }
            }
            MenuAction::CloseAll(index) => {
                for window in self.slots.get(index).map(|slot| slot.windows.clone()).unwrap_or_default() {
                    winlist::close(window.hwnd);
                }
            }
            MenuAction::Launch(index) => {
                if let Some(slot) = self.slots.get(index) {
                    if let Err(error) = apps::launch(&slot.target) { self.show_error(&error); }
                }
            }
            MenuAction::Pin(index) => {
                if let Some(slot) = self.slots.get(index).cloned() {
                    let executable = apps::resolve_shortcut(&slot.target).unwrap_or_else(|| slot.target.clone());
                    let label = (!slot.label.is_empty()).then(|| slot.label.clone());
                    if self.config.pin(Pin::new(slot.target.clone(), executable, label)) { self.save(); }
                }
            }
            MenuAction::Unpin(index) => {
                if let Some(slot) = self.slots.get(index).cloned() {
                    let executable = apps::resolve_shortcut(&slot.target).unwrap_or_else(|| slot.target.clone());
                    if self.config.unpin(&executable).is_some() || self.config.unpin(&slot.target).is_some() { self.save(); }
                }
            }
            MenuAction::Reveal(index) => {
                if let Some(slot) = self.slots.get(index) {
                    let target = apps::resolve_shortcut(&slot.target).unwrap_or_else(|| slot.target.clone());
                    if target.contains('\\') { apps::reveal(&target); }
                }
            }
            MenuAction::Launcher => self.open_launcher(),
            MenuAction::SetMagnification(enabled) => { self.config.magnification = enabled; self.apply_geometry(); }
            MenuAction::SetAutohide(enabled) => {
                self.config.autohide = enabled;
                if !enabled { self.reveal = 1.0; self.reveal_target = 1.0; }
                self.save();
                self.dirty = true;
            }
            MenuAction::SetClock(enabled) => { self.config.show_clock = enabled; self.save(); self.dirty = true; }
            MenuAction::SetStatus(enabled) => { self.config.show_status = enabled; self.apply_geometry(); }
            MenuAction::QuickSettings => tray::quick_settings(),
            MenuAction::Notifications => tray::notifications(),
            MenuAction::SetTrash(enabled) => { self.config.show_trash = enabled; self.save(); self.refresh(true); }
            MenuAction::IconSize(size) => { self.config.icon_size = size; self.apply_geometry(); }
            MenuAction::SetAutostart(enabled) => {
                match set_dock_autostart(enabled) {
                    Ok(()) => self.autostart = enabled,
                    Err(error) => self.show_error(&format!("{}", error.message())),
                }
            }
            MenuAction::OpenProfile => {
                if let Some(directory) = self.config_path.parent() { apps::reveal(&directory.to_string_lossy()); }
            }
            MenuAction::EmptyTrash => apps::empty_recycle_bin(),
            MenuAction::Exit => { let _ = unsafe { DestroyWindow(self.hwnd) }; }
        }
        self.dirty = true;
    }

    /// Applies a size or magnification change to the window geometry.
    fn apply_geometry(&mut self) {
        self.geometry = Geometry::new(&self.config, self.dpi);
        self.save();
        self.reposition();
        self.relayout();
        self.request_icons();
        self.dirty = true;
    }

    /// Draws the whole strip and presents it.
    fn paint(&mut self) {
        let width = (self.area.right - self.area.left).max(1);
        let height = self.geometry.window_height(&self.config).max(1);
        if self.surface.as_ref().is_none_or(|surface| surface.width != width || surface.height != height) {
            self.surface = Surface::new(width, height);
            // The Direct2D target keeps its size; bitmaps stay valid.
        }
        let palette = Palette::for_mode(self.dark, self.accent);
        let offset = layout::hidden_offset(self.reveal, self.geometry.metrics.panel, SLIVER);
        let panel = shift(self.placement.panel, offset);
        let items: Vec<(Rect, Rect)> = self.placement.items.iter().map(|item| {
            (shift(item.slot, offset), shift(item.icon, offset))
        }).collect();
        let slots = self.slots.clone();
        let hover = self.hover;
        let hover_label = hover.and_then(|index| slots.get(index))
            .map(|slot| match slot.kind {
                Kind::Launcher => zsdesk_core::i18n::text("全部应用").to_string(),
                Kind::Trash => zsdesk_core::i18n::text("回收站").to_string(),
                Kind::App => slot.label.clone(),
            })
            .filter(|label| !label.is_empty());
        let hover_icon = hover.and_then(|index| items.get(index)).map(|(_, icon)| *icon);
        let clock = self.clock.clone();
        let label_style = TextStyle::ui(self.geometry.label_size).weight(600).align(Align::Center);
        let clock_style = TextStyle::ui(self.geometry.clock_size).weight(600).align(Align::Center);
        let dot = self.geometry.dot;
        let radius = self.geometry.metrics.icon * 0.22;
        let show_clock = self.config.show_clock && !clock.is_empty();
        let separator = self.slots.iter().position(|slot| slot.kind != Kind::App)
            .and_then(|index| items.get(index)).map(|(slot, _)| slot.x - self.geometry.metrics.gap / 2.0);
        let shadow = palette.shadow;
        let accent = palette.accent;
        let dark = self.dark;
        let gfx = &self.gfx;
        let icons = &self.icons;
        let measured = hover_label.as_deref().map(|label| gfx.measure(label, &label_style, 260.0));
        let status_kinds = self.geometry.status_kinds(&self.config);
        let status_items = status::layout(panel, &status_kinds, self.geometry.status_item,
            self.geometry.status_gap, status::CLOCK);
        let snapshot = self.status;
        let hover_status = self.hover_status;
        let status_label = hover_status
            .map(|kind| zsdesk_core::i18n::text(kind.label()).to_string());
        let Some(surface) = self.surface.as_ref() else { return };
        let drawn = gfx.draw(surface, |canvas| {
            // Soft drop shadow under the panel.
            for step in 1..=3 {
                let grow = step as f32 * 2.0;
                canvas.round(RectF::new(panel.x - grow, panel.y + 2.0, panel.w + grow * 2.0, panel.h + grow * 0.5),
                    radius + grow, shadow.with_alpha(0.5 / step as f32));
            }
            canvas.round(rf(panel), radius, palette.panel);
            canvas.stroke_round(rf(panel), radius, palette.border, 1.0);
            for (index, (slot_box, icon_box)) in items.iter().enumerate() {
                let Some(slot) = slots.get(index) else { continue };
                match slot.kind {
                    Kind::App => match icons.bitmap(&slot.key) {
                        Some(bitmap) => canvas.image(bitmap, rf(*icon_box), 1.0),
                        None => {
                            canvas.round(rf(*icon_box), icon_box.w * 0.24, palette.secondary.with_alpha(0.22));
                            let initial = slot.label.chars().next()
                                .map(|value| value.to_uppercase().to_string()).unwrap_or_default();
                            canvas.text(&initial, rf(*icon_box),
                                &TextStyle::ui(icon_box.h * 0.42).weight(600).align(Align::Center), palette.text);
                        }
                    },
                    Kind::Launcher => draw_grid(canvas, *icon_box, palette.text),
                    Kind::Trash => canvas.glyph("\u{E74D}", rf(*icon_box), icon_box.h * 0.78, palette.text),
                }
                if slot.is_running() {
                    let center = RectF::new(slot_box.x + (slot_box.w - dot) / 2.0,
                        slot_box.bottom() - dot - 2.0, dot, dot);
                    let color = if hover == Some(index) { accent } else { palette.dot };
                    canvas.round(center, dot / 2.0, color);
                }
                if hover == Some(index) && slot.kind == Kind::App {
                    // macOS highlights the hovered slot with a faint plate.
                    canvas.round(rf(*icon_box).inset(-3.0, -3.0), icon_box.w * 0.26, palette.secondary.with_alpha(0.10));
                }
            }
            if let Some(x) = separator {
                canvas.fill(RectF::new(x, panel.y + panel.h * 0.22, 1.0, panel.h * 0.56),
                    palette.border.with_alpha(1.6));
            }
            for (kind, rect) in &status_items {
                if hover_status == Some(*kind) {
                    canvas.round(RectF::new(rect.x - 1.0, rect.y - 1.0, rect.w + 2.0, rect.h + 2.0),
                        rect.w * 0.32, palette.secondary.with_alpha(0.14));
                }
                draw_status(canvas, *kind, *rect, snapshot, palette.text, palette.secondary, accent);
            }
            if let (Some(label), Some(kind)) = (status_label.as_deref(), hover_status) {
                if let Some((_, rect)) = status_items.iter().find(|(item, _)| *item == kind) {
                    let (text_width, text_height) = gfx.measure(label, &label_style, 240.0);
                    let pill_width = text_width + 16.0;
                    let pill_height = text_height.max(16.0) + 8.0;
                    let x = (rect.x + rect.w / 2.0 - pill_width / 2.0)
                        .clamp(panel.x + 2.0, panel.right() - pill_width - 2.0);
                    let y = (rect.y - pill_height - 6.0).max(0.0);
                    canvas.round(RectF::new(x, y, pill_width, pill_height), pill_height / 2.0, palette.pill);
                    if dark {
                        canvas.stroke_round(RectF::new(x, y, pill_width, pill_height), pill_height / 2.0,
                            palette.border, 1.0);
                    }
                    canvas.text(label, RectF::new(x, y, pill_width, pill_height), &label_style, palette.text);
                }
            }
            if show_clock {
                let text_rect = RectF::new(panel.right() - 96.0, panel.y, 88.0, panel.h);
                canvas.text(&clock, text_rect, &clock_style, palette.text);
            }
            if let (Some(label), Some((text_width, text_height))) = (hover_label.as_deref(), measured) {
                if let Some(icon_box) = hover_icon {
                    let pill_width = text_width + 16.0;
                    let pill_height = text_height.max(16.0) + 8.0;
                    let x = (icon_box.x + icon_box.w / 2.0 - pill_width / 2.0)
                        .clamp(panel.x + 2.0, panel.right() - pill_width - 2.0);
                    let y = (icon_box.y - pill_height - 6.0).max(0.0);
                    canvas.round(RectF::new(x, y, pill_width, pill_height), pill_height / 2.0, palette.pill);
                    if dark { canvas.stroke_round(RectF::new(x, y, pill_width, pill_height), pill_height / 2.0,
                        palette.border, 1.0); }
                    canvas.text(label, RectF::new(x, y, pill_width, pill_height), &label_style, palette.text);
                }
            }
        });
        if drawn.is_err() { return; }
        let Some(surface) = self.surface.as_ref() else { return };
        let screen = unsafe { GetDC(None) };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8 };
        let position = POINT { x: self.area.left, y: self.area.bottom - height };
        let size = SIZE { cx: width, cy: height };
        let origin = POINT::default();
        let _ = unsafe { UpdateLayeredWindow(self.hwnd, Some(screen), Some(&position), Some(&size),
            Some(surface.dc), Some(&origin), Default::default(), Some(&blend), ULW_ALPHA) };
        unsafe { ReleaseDC(None, screen) };
    }
}

/// Menu entries resolved into an action so the menu can be destroyed first.
#[derive(Clone, Copy, Debug)]
enum MenuAction {
    Activate(isize),
    MinimizeAll(usize),
    CloseAll(usize),
    Launch(usize),
    Pin(usize),
    Unpin(usize),
    Reveal(usize),
    Launcher,
    SetMagnification(bool),
    SetAutohide(bool),
    SetClock(bool),
    SetStatus(bool),
    QuickSettings,
    Notifications,
    SetTrash(bool),
    IconSize(f32),
    SetAutostart(bool),
    OpenProfile,
    EmptyTrash,
    Exit,
}

fn append(menu: &HMENU, label: &str, flags: windows::Win32::UI::WindowsAndMessaging::MENU_ITEM_FLAGS, id: u32) {
    let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
    let _ = unsafe { AppendMenuW(*menu, flags, id as usize, PCWSTR(wide.as_ptr())) };
}

/// Paints one status indicator. Glyphs come from the Fluent icon font; the
/// battery is drawn by hand so the charge level stays crisp at every DPI.
fn draw_status(canvas: &Canvas, kind: status::Kind, rect: Rect, snapshot: tray::Snapshot,
    text: Color, dim: Color, accent: Color) {
    let size = rect.h * 0.66;
    let box_ = RectF::new(rect.x + (rect.w - size) / 2.0, rect.y + (rect.h - size) / 2.0, size, size);
    match kind {
        status::Kind::Chevron => canvas.glyph("\u{E70E}", box_, size, dim),
        status::Kind::Network => canvas.glyph("\u{E701}", box_, size,
            if snapshot.online { text } else { dim.with_alpha(0.55) }),
        status::Kind::Volume => canvas.glyph(if snapshot.muted { "\u{E74F}" } else { "\u{E767}" },
            box_, size, if snapshot.muted { dim } else { text }),
        status::Kind::Battery => {
            let body_width = size * 0.95;
            let body_height = size * 0.46;
            let x = rect.x + (rect.w - body_width - 2.0) / 2.0;
            let y = rect.y + (rect.h - body_height) / 2.0;
            let stroke = if snapshot.battery.is_some() { text } else { dim };
            canvas.stroke_round(RectF::new(x, y, body_width, body_height), body_height * 0.32,
                stroke.with_alpha(0.78), 1.2);
            canvas.round(RectF::new(x + body_width + 1.4, y + body_height * 0.3, 1.7, body_height * 0.4),
                0.8, stroke.with_alpha(0.6));
            if let Some(level) = snapshot.battery {
                let inner = ((body_width - 3.6) * (level as f32 / 100.0)).max(1.4);
                let low = level <= 20 && !snapshot.charging;
                canvas.round(RectF::new(x + 1.8, y + 1.8, inner, body_height - 3.6),
                    (body_height - 3.6) * 0.3, if low { Color::rgba8(255, 159, 107, 255) } else { text });
                if snapshot.charging {
                    // A short accent tick above the body marks mains power.
                    canvas.round(RectF::new(x + body_width / 2.0 - 1.0, y - body_height * 0.55, 2.0,
                        body_height * 0.34), 1.0, accent);
                }
            }
        }
    }
}

fn rf(rect: Rect) -> RectF { RectF::new(rect.x, rect.y, rect.w, rect.h) }

fn shift(rect: Rect, offset: f32) -> Rect { Rect::new(rect.x, rect.y + offset, rect.w, rect.h) }

/// The Launchpad-style 3 × 3 tile grid drawn for the all-applications icon.
fn draw_grid(canvas: &Canvas<'_>, box_rect: Rect, color: Color) {
    let cell = box_rect.w * 0.24;
    let gap = box_rect.w * 0.10;
    let total = cell * 3.0 + gap * 2.0;
    let x = box_rect.x + (box_rect.w - total) / 2.0;
    let y = box_rect.y + (box_rect.h - total) / 2.0;
    for row in 0..3 {
        for column in 0..3 {
            canvas.round(RectF::new(x + column as f32 * (cell + gap), y + row as f32 * (cell + gap), cell, cell),
                cell * 0.3, color);
        }
    }
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    POINT { x, y }
}

fn elide(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit { return text.to_string(); }
    let mut shortened: String = text.chars().take(limit.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

/// `HH:MM`, with a 12-hour suffix when the user asked for it.
pub fn clock_text(twenty_four: bool) -> String {
    let time: SYSTEMTIME = unsafe { GetLocalTime() };
    if twenty_four {
        format!("{:02}:{:02}", time.wHour, time.wMinute)
    } else {
        let hour = time.wHour % 12;
        let hour = if hour == 0 { 12 } else { hour };
        let suffix = if time.wHour < 12 { "AM" } else { "PM" };
        format!("{hour}:{:02} {suffix}", time.wMinute)
    }
}

/// Work area of the monitor that holds the pointer, falling back to the
/// primary monitor.
pub fn cursor_work_area() -> RECT { work_area(cursor_point().unwrap_or(POINT { x: 0, y: 0 })) }

fn cursor_point() -> Option<POINT> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.is_ok().then_some(point)
}

fn work_area(point: POINT) -> RECT {
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() { info.rcWork } else {
        RECT { left: 0, top: 0, right: 1024, bottom: 768 }
    }
}

fn dock_autostart_enabled() -> bool {
    let Ok(executable) = std::env::current_exe() else { return false };
    let expected = format!("\"{}\"", executable.display());
    zsdesk_windows::desktop::autostart_command(AUTOSTART_VALUE).ok().flatten()
        .is_some_and(|current| zsdesk_windows::desktop::autostart_owned(&current, &expected))
}

fn set_dock_autostart(enabled: bool) -> windows::core::Result<()> {
    let executable = std::env::current_exe().map_err(|error| {
        windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), error.to_string())
    })?;
    let expected = format!("\"{}\"", executable.display());
    zsdesk_windows::desktop::set_autostart(AUTOSTART_VALUE, &expected, enabled)
}

