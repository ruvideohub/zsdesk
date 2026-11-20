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
    RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WM_CONTEXTMENU, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_RBUTTONUP, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::config::ClockCorner;
use crate::desktop_clock::app_in_front;
use crate::gfx::{Gfx, TextStyle};
use crate::ui::{self, theme, v3};

/// The card at its normal size, in device-independent pixels.
const CARD_W: f32 = 268.0;
const CARD_H: f32 = 168.0;
/// Distance from the corner of the screen.
const MARGIN: f32 = 24.0;
/// Where the three rows sit, and how tall a meter bar is.
const ROWS: [f32; 3] = [46.0, 90.0, 134.0];
const BAR_H: f32 = 6.0;

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
    _target: DesktopWindowTarget,
    /// The panel's window: where the card posts its menu and drag requests.
    dock: HWND,
    monitor: RECT,
    dpi: f32,
    prefs: Prefs,
    /// The meters last drawn, so a tick that changes nothing costs nothing.
    drawn: Vec<Meter>,
    /// The shared system sampler: the same numbers the panel's monitor widget shows.
    sampler: laminar_sys::stats::Sampler,
    /// The first reading has no delta behind it and is drawn as «…».
    sampled: bool,
    /// Pointer offset inside the card while dragging.
    drag: Option<(i32, i32)>,
    /// The app window the card was last pushed under (0: none).
    below: isize,
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
            // Like the clock: under the app in front, never over it.
            match app_in_front() {
                Some(front) => {
                    let _ = SetWindowPos(self.hwnd, Some(front), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                }
                None => {
                    let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOZORDER);
                }
            }
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    /// Keep the card under the app in front of it (once a second, while it is visible).
    fn keep_below(&mut self) {
        let Some(front) = app_in_front() else {
            self.below = 0;
            return;
        };
        if self.below == front.0 as isize {
            return;
        }
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(front), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        self.below = front.0 as isize;
    }

    /// Read the three numbers. The processor needs two samples, so the first tick draws it
    /// as «…» and the second one shows the truth.
    fn measure(&mut self) -> Vec<Meter> {
        let stats = self.sampler.sample();
        let first = !self.sampled;
        self.sampled = true;
        let cpu_fill = (stats.cpu_percent / 100.0).clamp(0.0, 1.0);
        let cpu = Meter { label: "ЦП", fill: cpu_fill, text: if first { "…".to_string() } else { percent(cpu_fill) } };
        let memory = if stats.mem_total_gb > 0.0 {
            let fill = (stats.mem_used_gb / stats.mem_total_gb).clamp(0.0, 1.0);
            Meter { label: "Память", fill, text: format!("{} из {} ГБ", decimal(stats.mem_used_gb), decimal(stats.mem_total_gb)) }
        } else {
            Meter { label: "Память", fill: 0.0, text: "нет данных".to_string() }
        };
        let disk_fill = (stats.disk_c_used_percent / 100.0).clamp(0.0, 1.0);
        let disk = Meter { label: "Диск", fill: disk_fill, text: format!("занято {}", percent(disk_fill)) };
        vec![cpu, memory, disk]
    }
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
    pub fn new(c: &Compositor, monitor: RECT, dpi: f32, prefs: Prefs, dock: HWND) -> Result<Self> {
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
                _target: target,
                dock,
                monitor,
                dpi,
                prefs,
                drawn: Vec::new(),
                sampler: laminar_sys::stats::Sampler::new(),
                sampled: false,
                drag: None,
                below: 0,
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
        self.state.keep_below();
        let meters = self.state.measure();
        if meters == self.state.drawn {
            return Ok(());
        }

        let k = self.state.k();
        let (card_w, card_h) = (CARD_W * k, CARD_H * k);
        let root = &self.state.root;
        root.Children()?.RemoveAll()?;

        let (bg, _, _) = ui::rounded(c, card_w, card_h, 26.0 * k, theme::surface(0.55))?;
        root.Children()?.InsertAtTop(&bg)?;

        let (title, _, _) = ui::text(
            c,
            gfx,
            "Система",
            &TextStyle { px: 13.0 * k, weight: 600, color: theme::text_2(), ..Default::default() },
        )?;
        title.SetOffset(v3(24.0 * k, 16.0 * k, 0.0))?;
        root.Children()?.InsertAtTop(&title)?;

        let bar_w = card_w - 48.0 * k;
        for (index, meter) in meters.iter().enumerate() {
            let y = ROWS[index] * k;
            let (label, label_w, _) = ui::text(
                c,
                gfx,
                meter.label,
                &TextStyle { px: 13.0 * k, weight: 500, ..Default::default() },
            )?;
            label.SetOffset(v3(24.0 * k, y, 0.0))?;
            root.Children()?.InsertAtTop(&label)?;

            // The number sits to the left of the bar's right end, out of the label's way.
            let (value, value_w, _) = ui::text(
                c,
                gfx,
                &meter.text,
                &TextStyle { px: 12.5 * k, weight: 500, color: theme::text_2(), ..Default::default() },
            )?;
            let value_x = (24.0 * k + label_w + 8.0 * k).min(24.0 * k + bar_w - value_w);
            value.SetOffset(v3(value_x, y, 0.0))?;
            root.Children()?.InsertAtTop(&value)?;

            // The track, then the fill on top of it.
            let (track, _, _) = ui::rounded(c, bar_w, BAR_H * k, BAR_H * k / 2.0, theme::ink(0.12))?;
            track.SetOffset(v3(24.0 * k, y + 20.0 * k, 0.0))?;
            root.Children()?.InsertAtTop(&track)?;
            let filled = (bar_w * meter.fill).max(BAR_H * k);
            if meter.fill > 0.0 {
                // A nearly full card stays readable: red only when the meter is really full.
                let (fill, _, _) = ui::rounded(c, filled, BAR_H * k, BAR_H * k / 2.0, theme::accent())?;
                fill.SetOffset(v3(24.0 * k, y + 20.0 * k, 0.0))?;
                root.Children()?.InsertAtTop(&fill)?;
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
                        crate::dock::WM_APP_SYSTEM_MENU,
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
                            crate::dock::WM_APP_SYSTEM_MOVED,
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
    fn the_card_keeps_a_widget_shape() {
        assert!(CARD_W > CARD_H);
        assert!(CARD_W / CARD_H < 2.5);
        // Three rows fit inside the card with the title above them.
        assert!(ROWS[2] + 26.0 < CARD_H);
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
