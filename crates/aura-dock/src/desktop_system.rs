//! The desktop load card: processor, memory and disk, as three quiet meters on one glass
//! card — the second desktop widget, next to the clock.
//!
//! It is built exactly like the clock card (`desktop_clock.rs`): a window of its own process
//! (`widgets.exe --component system`), never topmost, never taking the keyboard focus, sitting
//! above the wallpaper and below application windows. It is its own settings dialog too:
//! right-click asks the panel for the card's menu, dragging moves it, and the panel — the one
//! writer of the settings — saves the spot and restarts this process.
//!
//! The numbers come from the system, not from a service — `laminar_sys::stats` already samples
//! them for the panel's own monitor widget (processor deltas, memory, the system drive), so
//! the card and the widget can never disagree about what the machine is doing. The first
//! reading has no delta behind it, so it is drawn as «…» rather than as a made-up zero.

use windows::core::{w, Interface, Result};
use windows::UI::Composition::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_EXCLUDED_FROM_PEEK, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowLongPtrW, GetWindowRect,
    RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WM_CONTEXTMENU, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_RBUTTONUP, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::config::ClockCorner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { System, Network, Battery }

impl Kind {
    fn title(self) -> &'static str {
        match self { Self::System => "Система", Self::Network => "Сеть", Self::Battery => "Батарея" }
    }
    fn menu_message(self) -> u32 {
        match self { Self::System => crate::dock::WM_APP_SYSTEM_MENU, Self::Network => crate::dock::WM_APP_NETWORK_MENU, Self::Battery => crate::dock::WM_APP_BATTERY_MENU }
    }
    fn moved_message(self) -> u32 {
        match self { Self::System => crate::dock::WM_APP_SYSTEM_MOVED, Self::Network => crate::dock::WM_APP_NETWORK_MOVED, Self::Battery => crate::dock::WM_APP_BATTERY_MOVED }
    }
}

use crate::gfx::Gfx;
use crate::ui::theme;

/// The card at its normal size, in device-independent pixels.
const CARD_W: f32 = 268.0;
const CARD_H: f32 = 168.0;
/// Distance from the corner of the screen.
const MARGIN: f32 = 24.0;
/// Where the three rows sit, and how tall a meter bar is.
const ROWS: [f32; 3] = [62.0, 94.0, 126.0];
const BAR_H: f32 = 4.0;
const NETWORK_TILE_X: [f32; 2] = [18.0, 137.0];
const NETWORK_TILE_W: f32 = 113.0;
const NETWORK_TILE_Y: f32 = 62.0;
const NETWORK_TILE_H: f32 = 84.0;

const CLASS: windows::core::PCWSTR = w!("AuraExplorerDesktopSystem");

/// What the card looks like: everything the card's own menu changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    pub corner: ClockCorner,
    /// 0.75–1.5 multiplier on the whole card.
    pub scale: f32,
    /// Where the card was left, in physical px; `None` means its corner.
    pub pos: Option<(i32, i32)>,
    /// Dragging does nothing while this is on.
    pub locked: bool,
}

impl Prefs {
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        Self {
            corner: cfg.system_corner,
            scale: cfg.system_scale,
            pos: cfg.system_pos.map(|[x, y]| (x, y)),
            locked: cfg.system_locked,
        }
    }

    pub fn for_kind(cfg: &crate::config::Config, kind: Kind) -> Self {
        match kind {
            Kind::System => Self::from_config(cfg),
            Kind::Network => Self { corner: ClockCorner::TopLeft, scale: 1.0, pos: cfg.network_pos.map(|[x,y]| (x,y)), locked: cfg.network_locked },
            Kind::Battery => Self { corner: ClockCorner::BottomLeft, scale: 1.0, pos: cfg.battery_pos.map(|[x,y]| (x,y)), locked: cfg.battery_locked },
        }
    }

    fn scale_ok(&self) -> f32 {
        self.scale.clamp(0.75, 1.5)
    }
}

/// One meter: what it is and how full it is, 0.0–1.0, and the number to print.
#[derive(Clone, Debug, PartialEq)]
pub struct Meter {
    pub label: &'static str,
    /// 0.0–1.0; negative means "no reading yet" (the first second of the processor).
    pub fill: f32,
    pub text: String,
}

/// "23%": a whole percent, the way a person reads a meter.
fn percent(fill: f32) -> String {
    format!("{}%", (fill * 100.0).round() as i32)
}

/// One decimal, the Russian way: «7,8».
fn decimal(value: f32) -> String {
    format!("{value:.1}").replace('.', ",")
}

/// Everything the window procedure touches. Boxed, so the address in `GWLP_USERDATA`
/// survives the value being moved into the caller's state.
struct SystemState {
    hwnd: HWND,
    root: ContainerVisual,
    backdrop: Option<CompositionBrush>,
    _target: DesktopWindowTarget,
    /// The panel's window: where the card posts its menu and drag requests.
    dock: HWND,
    monitor: RECT,
    dpi: f32,
    prefs: Prefs,
    kind: Kind,
    /// The meters last drawn, so a tick that changes nothing costs nothing.
    drawn: Vec<Meter>,
    /// The shared system sampler: the same numbers the panel's monitor widget shows.
    sampler: laminar_sys::stats::Sampler,
    /// The first reading has no delta behind it and is drawn as «…».
    sampled: bool,
    /// Pointer offset inside the card while dragging.
    drag: Option<(i32, i32)>,
}

impl SystemState {
    /// Screen pixels per logical pixel: the display's scale times the card's own size.
    fn k(&self) -> f32 {
        self.dpi.max(1.0) * self.prefs.scale_ok()
    }

    /// The card's size in physical pixels.
    fn size(&self) -> (i32, i32) {
        let k = self.k();
        ((CARD_W * k).round() as i32, (CARD_H * k).round() as i32)
    }

    /// Move the card into the corner the settings ask for.
    fn place(&self) {
        let (w, h) = self.size();
        let margin = (MARGIN * self.dpi.max(1.0)).round() as i32;
        let (x, y) = match self.prefs.pos {
            Some((px, py)) => (
                px.clamp(self.monitor.left, (self.monitor.right - w).max(self.monitor.left)),
                py.clamp(self.monitor.top, (self.monitor.bottom - h).max(self.monitor.top)),
            ),
            None => corner_origin(self.prefs.corner, self.monitor, w, h, margin),
        };
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOZORDER);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        crate::desktop_layer::keep_below_windows(self.hwnd);
    }

    /// Read the three numbers. The processor needs two samples, so the first tick draws it
    /// as «…» and the second one shows the truth.
    fn measure(&mut self) -> Vec<Meter> {
        let stats = self.sampler.sample();
        let first = !self.sampled;
        self.sampled = true;
        match self.kind {
            Kind::System => meters(&stats, first),
            Kind::Network => network_meters(&stats, first),
            Kind::Battery => battery_meters(),
        }
    }
}

/// Convert real readings to visible meters without treating a failed query as zero.
fn meters(stats: &laminar_sys::stats::Stats, first: bool) -> Vec<Meter> {
    let cpu_fill = (stats.cpu_percent / 100.0).clamp(0.0, 1.0);
    let cpu = if first {
        Meter { label: "ЦП", fill: 0.0, text: "…".to_string() }
    } else if stats.cpu_valid {
        Meter { label: "ЦП", fill: cpu_fill, text: percent(cpu_fill) }
    } else {
        Meter { label: "ЦП", fill: 0.0, text: "нет данных".to_string() }
    };
    let memory = if stats.mem_total_gb > 0.0 {
        let fill = (stats.mem_used_gb / stats.mem_total_gb).clamp(0.0, 1.0);
        Meter { label: "Память", fill, text: format!("{} из {} ГБ", decimal(stats.mem_used_gb), decimal(stats.mem_total_gb)) }
    } else {
        Meter { label: "Память", fill: 0.0, text: "нет данных".to_string() }
    };
    let disk = match stats.disk_c_free_gb {
        Some(free) => Meter { label: "Диск C:", fill: (stats.disk_c_used_percent / 100.0).clamp(0.0, 1.0), text: format!("свободно {} ГБ", decimal(free)) },
        None => Meter { label: "Диск C:", fill: 0.0, text: "нет данных".to_string() },
    };
    vec![cpu, memory, disk]

}

fn rate(bytes: f64) -> String {
    if bytes >= 1_000_000.0 { format!("{:.1} МБ/с", bytes / 1_000_000.0) }
    else if bytes >= 1_000.0 { format!("{:.0} КБ/с", bytes / 1_000.0) }
    else { format!("{:.0} Б/с", bytes.max(0.0)) }
}

fn network_meters(stats: &laminar_sys::stats::Stats, first: bool) -> Vec<Meter> {
    let display = |n| if !first && stats.net_valid { rate(n) } else { "нет данных".to_string() };
    vec![Meter { label: "Получение", fill: 0.0, text: display(stats.net_down_bps) },
         Meter { label: "Отправка", fill: 0.0, text: display(stats.net_up_bps) }]
}

fn battery_meters() -> Vec<Meter> {
    let feed = match laminar_sys::connectors::local::battery() {
        Ok(feed) => feed,
        Err(laminar_sys::feed::FeedError::NeedsSetup(_)) => return vec![Meter { label: "Батарея", fill: 0.0, text: "нет батареи".into() }],
        Err(_) => return vec![Meter { label: "Батарея", fill: 0.0, text: "нет данных".into() }],
    };
    let fill = feed.frac.unwrap_or(0.0).clamp(0.0, 1.0);
    let status = if feed.extra.get("charging").map(String::as_str) == Some("true") { "заряжается" }
        else if feed.extra.get("plugged_in").map(String::as_str) == Some("true") { "от сети" }
        else if feed.extra.get("power_known").map(String::as_str) == Some("true") { "от батареи" }
        else { "неизвестно" };
    let left = feed.extra.get("time_left").filter(|v| !v.is_empty()).cloned().unwrap_or_else(|| "—".into());
    vec![Meter { label: "Заряд", fill, text: percent(fill) },
         Meter { label: "Питание", fill: 0.0, text: status.into() },
         Meter { label: "Осталось", fill: 0.0, text: left }]
}

/// Top-left of a card of `w`×`h` in `corner` of `monitor`, `margin` px from the edges.
fn corner_origin(corner: ClockCorner, monitor: RECT, w: i32, h: i32, margin: i32) -> (i32, i32) {
    let left = monitor.left + margin;
    let right = monitor.right - w - margin;
    let top = monitor.top + margin;
    let bottom = monitor.bottom - h - margin;
    match corner {
        ClockCorner::TopLeft => (left, top),
        ClockCorner::TopRight => (right, top),
        ClockCorner::BottomLeft => (left, bottom),
        ClockCorner::BottomRight => (right, bottom),
    }
}

pub struct DesktopSystem {
    state: Box<SystemState>,
}

impl DesktopSystem {
    /// Create the card on `monitor`, or return an error the caller can simply ignore:
    /// a desktop without this card is fine, a desktop that fails to start is not.
    pub fn new(c: &Compositor, monitor: RECT, dpi: f32, prefs: Prefs, dock: HWND, kind: Kind) -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassW(&class);
            let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            let hwnd = CreateWindowExW(
                ex,
                CLASS,
                w!("AuraUI Desktop Load"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let on: i32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &on as *const _ as _, 4);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_EXCLUDED_FROM_PEEK, &on as *const _ as _, 4);
            let target = c.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, true)?;
            let root = c.CreateContainerVisual()?;
            target.SetRoot(&root)?;
            let state = Box::new(SystemState {
                hwnd,
                root,
                backdrop: crate::desktop_style::backdrop(c),
                _target: target,
                dock,
                monitor,
                dpi,
                prefs,
                kind,
                drawn: Vec::new(),
                sampler: laminar_sys::stats::Sampler::new(),
                sampled: false,
                drag: None,
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*state as *const SystemState as isize);
            let card = Self { state };
            card.state.place();
            Ok(card)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.state.hwnd
    }

    #[allow(dead_code)]
    pub fn prefs(&self) -> Prefs {
        self.state.prefs
    }

    /// Apply new settings: move, resize and repaint if anything changed.
    pub fn set_prefs(&mut self, prefs: Prefs) {
        if self.state.prefs == prefs {
            return;
        }
        self.state.prefs = prefs;
        self.state.drawn.clear();
        self.state.place();
    }

    /// Draw the card now, whatever was drawn before.
    pub fn repaint(&mut self, c: &Compositor, gfx: &Gfx) -> Result<()> {
        self.state.drawn.clear();
        self.tick(c, gfx)
    }

    /// Draw the card. Called on a one-second timer; redraws only when a number changed.
    pub fn tick(&mut self, c: &Compositor, gfx: &Gfx) -> Result<()> {
        crate::desktop_layer::keep_below_windows(self.state.hwnd);
        let meters = self.state.measure();
        if meters == self.state.drawn {
            return Ok(());
        }

        let k = self.state.k();
        let (card_w, card_h) = (CARD_W * k, CARD_H * k);
        let root = &self.state.root;
        root.Children()?.RemoveAll()?;

        use crate::desktop_style as style;
        style::surface(c, root, self.state.backdrop.as_ref(), card_w, card_h, k)?;
        let (subtitle, glyph) = match self.state.kind {
            Kind::System => ("НАГРУЗКА УСТРОЙСТВА", "\u{E7F4}"),
            Kind::Network => ("СКОРОСТЬ СОЕДИНЕНИЯ", "\u{E701}"),
            Kind::Battery => ("ПИТАНИЕ УСТРОЙСТВА", "\u{E83E}"),
        };
        style::header(c, gfx, root, self.state.kind.title(), subtitle, glyph, k)?;

        match self.state.kind {
            Kind::System => {
                for (index, meter) in meters.iter().enumerate() {
                    let y = ROWS[index] * k;
                    style::text(c, gfx, root, meter.label, 21.0*k, y, 74.0*k, 11.0*k, 500, theme::text_2(), false, false)?;
                    style::text(c, gfx, root, &meter.text, 100.0*k, y, 147.0*k, 11.0*k, 600, theme::text(), false, true)?;
                    style::rect(c, root, 21.0*k, y + 19.0*k, 226.0*k, BAR_H*k, 2.0*k, theme::ink(0.11))?;
                    if meter.fill > 0.0 {
                        style::rect(c, root, 21.0*k, y + 19.0*k, (226.0*k*meter.fill).max(4.0*k), BAR_H*k, 2.0*k, theme::accent())?;
                    }
                }
            }
            Kind::Network => {
                // Throughput has no honest 0–100% scale: use two equal value tiles,
                // not empty progress bars that would suggest a connection limit.
                for (i, meter) in meters.iter().enumerate() {
                    let x = NETWORK_TILE_X[i] * k;
                    style::rect(c, root, x, NETWORK_TILE_Y*k, NETWORK_TILE_W*k, NETWORK_TILE_H*k, 16.0*k, theme::ink(0.065))?;
                    let label = if i == 0 { "↓  ПОЛУЧЕНИЕ" } else { "↑  ОТПРАВКА" };
                    style::text(c, gfx, root, label, x + 12.0*k, 73.0*k, 91.0*k, 9.0*k, 600, theme::text_2(), false, false)?;
                    style::fitted_text(c, gfx, root, &meter.text, x + 12.0*k, 101.0*k, 91.0*k, 15.0*k, 600, theme::text(), true)?;
                }
            }
            Kind::Battery => {
                if let Some(charge) = meters.first() {
                    let has_details = meters.len() == 3;
                    style::text(c, gfx, root, &charge.text, 21.0*k, 63.0*k, 118.0*k,
                        if has_details { 29.0*k } else { 15.0*k }, 600, theme::text(), true, false)?;
                    if has_details {
                        style::text(c, gfx, root, &meters[1].text, 22.0*k, 103.0*k, 116.0*k,
                            11.0*k, 500, theme::text_2(), false, false)?;
                        style::rect(c, root, 151.0*k, 65.0*k, 97.0*k, 55.0*k, 15.0*k, theme::ink(0.065))?;
                        style::text(c, gfx, root, "ОСТАЛОСЬ", 162.0*k, 72.0*k, 76.0*k, 9.0*k, 600, theme::text_2(), false, false)?;
                        style::text(c, gfx, root, &meters[2].text, 162.0*k, 91.0*k, 76.0*k, 12.0*k, 600, theme::text(), false, false)?;
                        style::rect(c, root, 21.0*k, 138.0*k, 226.0*k, 5.0*k, 2.5*k, theme::ink(0.11))?;
                        if charge.fill > 0.0 {
                            style::rect(c, root, 21.0*k, 138.0*k, (226.0*k*charge.fill).max(5.0*k), 5.0*k, 2.5*k, theme::accent())?;
                        }
                    }
                }
            }
        }

        self.state.drawn = meters;
        Ok(())
    }

    pub fn destroy(&mut self) {
        if !self.state.hwnd.0.is_null() {
            unsafe {
                let _ = DestroyWindow(self.state.hwnd);
            }
            self.state.hwnd = HWND(std::ptr::null_mut());
        }
    }
}

impl Drop for DesktopSystem {
    fn drop(&mut self) {
        self.destroy();
    }
}

fn state_of(hwnd: HWND) -> Option<&'static mut SystemState> {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut SystemState;
    unsafe { raw.as_mut() }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // Right-click (and the keyboard menu key) opens the card's menu, drawn by the panel.
        WM_RBUTTONUP | WM_CONTEXTMENU => {
            if let Some(state) = state_of(hwnd) {
                let _ = unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(state.dock),
                        state.kind.menu_message(),
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Some(state) = state_of(hwnd) {
                if state.prefs.locked {
                    return LRESULT(0);
                }
                let mut pointer = POINT::default();
                let mut card = RECT::default();
                unsafe {
                    let _ = GetCursorPos(&mut pointer);
                    let _ = GetWindowRect(hwnd, &mut card);
                    state.drag = Some((pointer.x - card.left, pointer.y - card.top));
                    let _ = SetCapture(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(state) = state_of(hwnd) {
                if let Some((grab_x, grab_y)) = state.drag {
                    let mut pointer = POINT::default();
                    unsafe {
                        let _ = GetCursorPos(&mut pointer);
                    }
                    let (w, h) = state.size();
                    let x = (pointer.x - grab_x).clamp(state.monitor.left, state.monitor.right - w);
                    let y = (pointer.y - grab_y).clamp(state.monitor.top, state.monitor.bottom - h);
                    unsafe {
                        let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(state) = state_of(hwnd) {
                if state.drag.take().is_some() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    let mut rect = RECT::default();
                    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
                    state.prefs.pos = Some((rect.left, rect.top));
                    state.place();
                    // The panel owns the settings: tell it where the card was left.
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                            Some(state.dock),
                            state.kind.moved_message(),
                            WPARAM(rect.left as isize as usize),
                            LPARAM(rect.top as isize),
                        );
                    }
                }
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_readings_distinguish_unknown_from_idle() {
        let mut s = laminar_sys::stats::Stats::default();
        assert_eq!(network_meters(&s, false)[0].text, "нет данных");
        s.net_valid = true;
        assert_eq!(network_meters(&s, true)[0].text, "нет данных");
        assert_eq!(network_meters(&s, false)[0].text, "0 Б/с");
        s.net_down_bps = 2_000_000.0;
        assert_eq!(network_meters(&s, false)[0].text, "2.0 МБ/с");
    }

    #[test]
    fn meters_distinguish_unavailable_from_a_real_zero() {
        let mut s = laminar_sys::stats::Stats::default();
        let unknown = meters(&s, false);
        assert_eq!(unknown[0].text, "нет данных");
        assert_eq!(unknown[1].text, "нет данных");
        assert_eq!(unknown[2].text, "нет данных");
        s.cpu_valid = true;
        s.disk_c_free_gb = Some(100.0);
        let measured = meters(&s, false);
        assert_eq!(measured[0].text, "0%");
        assert_eq!(measured[2].text, "свободно 100,0 ГБ");
        assert_eq!(meters(&s, true)[0].text, "…");
    }

    #[test]
    fn the_card_keeps_a_widget_shape() {
        assert!(CARD_W > CARD_H);
        assert!(CARD_W / CARD_H < 2.5);
        // Three rows fit inside the card with the title above them.
        assert!(ROWS[2] + 26.0 < CARD_H);
    }

    #[test]
    fn network_tiles_stay_inside_card_and_do_not_overlap_at_supported_scales() {
        for scale in [0.75_f32, 1.0, 1.25, 1.5, 2.0] {
            let left_end = (NETWORK_TILE_X[0] + NETWORK_TILE_W) * scale;
            let right_start = NETWORK_TILE_X[1] * scale;
            assert!(left_end < right_start);
            assert!((NETWORK_TILE_X[1] + NETWORK_TILE_W) * scale <= CARD_W * scale);
            assert!((NETWORK_TILE_Y + NETWORK_TILE_H) * scale <= CARD_H * scale);
        }
    }

    #[test]
    fn a_meter_prints_a_whole_percent() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.234), "23%");
        assert_eq!(percent(1.0), "100%");
    }

    #[test]
    fn decimals_read_the_russian_way() {
        assert_eq!(decimal(15.9375), "15,9");
        assert_eq!(decimal(0.0), "0,0");
    }

    #[test]
    fn the_four_corners_are_inside_the_monitor() {
        let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        let places: std::collections::BTreeSet<(i32, i32)> = ClockCorner::ALL
            .iter()
            .map(|corner| corner_origin(*corner, monitor, 268, 168, 24))
            .collect();
        assert_eq!(places.len(), 4);
        for (x, y) in places {
            assert!(x >= monitor.left && x + 268 <= monitor.right);
            assert!(y >= monitor.top && y + 168 <= monitor.bottom);
        }
    }
}
