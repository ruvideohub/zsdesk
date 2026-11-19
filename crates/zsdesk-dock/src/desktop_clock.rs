//! The desktop clock: a quiet glass card in a corner of the desktop, macOS style.
//!
//! It runs in the panel's process but not in the panel's window. It is an ordinary,
//! *not topmost* window, so it sits above the wallpaper and below application windows —
//! the way desktop widgets behave on a Mac.
//!
//! It is clickable, but never takes the keyboard focus (`WS_EX_NOACTIVATE`), because the
//! card is also its own settings dialog:
//!
//! - **right-click** opens the card's menu (corner, size, 24-hour time, date, seconds,
//!   hide) drawn by the panel, which owns the styled menu;
//! - **drag** moves it, and letting go snaps it to the nearest corner.
//!
//! Both changes are written to the panel's settings file by the panel (`crate::dock`),
//! never from here: this module only posts `WM_APP_CLOCK_MENU` / `WM_APP_CLOCK_MOVED`.
//!
//! The time refreshes once a second; nothing is redrawn unless the text changed.

use windows::core::{w, Interface, Result};
use windows::UI::Composition::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_EXCLUDED_FROM_PEEK, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowLongPtrW, GetWindowRect, RegisterClassW,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, SWP_NOACTIVATE, SWP_NOZORDER, SWP_NOSIZE,
    SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WM_CONTEXTMENU, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP,
    WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::config::ClockCorner;
use crate::gfx::{Gfx, TextStyle};
use crate::ui::{self, theme, v3};

/// The card at its normal size, in device-independent pixels.
const CARD_W: f32 = 268.0;
const CARD_H: f32 = 132.0;
/// The card without the date: time only.
const CARD_H_SHORT: f32 = 96.0;
/// Distance from the corner of the screen.
const MARGIN: f32 = 24.0;

const CLASS: windows::core::PCWSTR = w!("ZSDeskDesktopClock");

const WEEKDAYS: [&str; 7] = [
    "воскресенье",
    "понедельник",
    "вторник",
    "среда",
    "четверг",
    "пятница",
    "суббота",
];
const MONTHS: [&str; 12] = [
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];

/// What the card looks like: everything the card's own menu changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    pub corner: ClockCorner,
    /// 0.75–1.5 multiplier on the whole card.
    pub scale: f32,
    /// 12-hour time with AM/PM instead of 24-hour.
    pub hour12: bool,
    /// The weekday and date under the time.
    pub date: bool,
    /// Seconds in the time (the card then repaints every second).
    pub seconds: bool,
}

impl Prefs {
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        Self {
            corner: cfg.clock_corner,
            scale: cfg.clock_scale,
            hour12: cfg.clock_12h,
            date: cfg.clock_date,
            seconds: cfg.clock_seconds,
        }
    }

    fn scale_ok(&self) -> f32 {
        self.scale.clamp(0.75, 1.5)
    }
}

/// Everything the window procedure touches. Boxed, so the address in `GWLP_USERDATA`
/// survives the value being moved into the panel's state.
struct ClockState {
    hwnd: HWND,
    root: ContainerVisual,
    _target: DesktopWindowTarget,
    /// The panel's window: where the card posts its menu and drag requests.
    dock: HWND,
    monitor: RECT,
    dpi: f32,
    prefs: Prefs,
    /// The text last drawn, so a tick that changes nothing costs nothing.
    drawn: String,
    /// Pointer offset inside the card while dragging.
    drag: Option<(i32, i32)>,
}

impl ClockState {
    /// Screen pixels per logical pixel: the display's scale times the card's own size.
    fn k(&self) -> f32 {
        self.dpi.max(1.0) * self.prefs.scale_ok()
    }

    /// The card's size in physical pixels.
    fn size(&self) -> (i32, i32) {
        let k = self.k();
        let h = if self.prefs.date { CARD_H } else { CARD_H_SHORT };
        ((CARD_W * k).round() as i32, (h * k).round() as i32)
    }

    /// Move the card into the corner the settings ask for.
    fn place(&self) {
        let (w, h) = self.size();
        let margin = (MARGIN * self.dpi.max(1.0)).round() as i32;
        let (x, y) = corner_origin(self.prefs.corner, self.monitor, w, h, margin);
        unsafe {
            // Not HWND_TOPMOST: the card belongs to the desktop, not above your work.
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    /// The corner nearest to where the card was dropped.
    fn nearest_corner(&self) -> ClockCorner {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut r) }.is_err() {
            return self.prefs.corner;
        }
        let cx = (r.left + r.right) / 2;
        let cy = (r.top + r.bottom) / 2;
        let monitor = self.monitor;
        let middle_x = (monitor.left + monitor.right) / 2;
        let middle_y = (monitor.top + monitor.bottom) / 2;
        match (cx < middle_x, cy < middle_y) {
            (true, true) => ClockCorner::TopLeft,
            (false, true) => ClockCorner::TopRight,
            (true, false) => ClockCorner::BottomLeft,
            (false, false) => ClockCorner::BottomRight,
        }
    }

    /// The two halves of the time: the digits and, for 12-hour time, AM/PM.
    fn time_text(&self, now: &SYSTEMTIME) -> (String, String) {
        time_text(self.prefs, now)
    }

    fn date_text(&self, now: &SYSTEMTIME) -> String {
        date_text(now)
    }
}

/// The two halves of the time: the digits and, for 12-hour time, AM/PM.
fn time_text(prefs: Prefs, now: &SYSTEMTIME) -> (String, String) {
let seconds = prefs.seconds;
    if prefs.hour12 {
        let hour = match now.wHour % 12 {
            0 => 12,
            hour => hour,
        };
        let suffix = if now.wHour < 12 { "AM" } else { "PM" };
        let time = if seconds {
            format!("{hour}:{:02}:{:02}", now.wMinute, now.wSecond)
        } else {
            format!("{hour}:{:02}", now.wMinute)
        };
        (time, suffix.to_string())
    } else {
        let time = if seconds {
            format!("{:02}:{:02}:{:02}", now.wHour, now.wMinute, now.wSecond)
        } else {
            format!("{:02}:{:02}", now.wHour, now.wMinute)
        };
        (time, String::new())
    }
}

/// «среда, 19 ноября».
fn date_text(now: &SYSTEMTIME) -> String {
    format!(
        "{}, {} {}",
        WEEKDAYS[(now.wDayOfWeek as usize).min(6)],
        now.wDay,
        MONTHS[(now.wMonth.saturating_sub(1) as usize).min(11)]
    )
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

pub struct DesktopClock {
    state: Box<ClockState>,
}

impl DesktopClock {
    /// Create the card on `monitor`, or return an error the caller can simply ignore:
    /// a desktop without a clock is fine, a desktop that fails to start is not.
    pub fn new(c: &Compositor, monitor: RECT, dpi: f32, prefs: Prefs, dock: HWND) -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassW(&class);
            // No redirection bitmap (Composition draws) and no activation (a click on the
            // card must never take the focus away from what you are typing in). It *is*
            // hit-testable: right-click opens the card's menu and dragging moves it.
            let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            let hwnd = CreateWindowExW(
                ex,
                CLASS,
                w!("ZSDesk Desktop Clock"),
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
            let state = Box::new(ClockState {
                hwnd,
                root,
                _target: target,
                dock,
                monitor,
                dpi,
                prefs,
                drawn: String::new(),
                drag: None,
            });
            // The window procedure reaches its state through this pointer. `state` is
            // boxed, so the address stays valid when the panel moves the clock around.
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*state as *const ClockState as isize);
            let clock = Self { state };
            clock.state.place();
            Ok(clock)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.state.hwnd
    }

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

    /// Draw the card. Called on a one-second timer; redraws only when the text changed.
    pub fn tick(&mut self, c: &Compositor, gfx: &Gfx) -> Result<()> {
        let now = local_time();
        let (time, suffix) = self.state.time_text(&now);
        let date = if self.state.prefs.date { self.state.date_text(&now) } else { String::new() };
        let key = format!("{time}|{suffix}|{date}|{:.3}", self.state.k());
        if key == self.state.drawn {
            return Ok(());
        }

        let k = self.state.k();
        let (card_w, card_h) = card_size(self.state.prefs, k);
        let root = &self.state.root;
        root.Children()?.RemoveAll()?;

        // Glass, the same surface the panel and Launchpad use.
        let (bg, _, _) = ui::rounded(c, card_w, card_h, 26.0 * k, theme::surface(0.55))?;
        root.Children()?.InsertAtTop(&bg)?;

        // Seconds and AM/PM both make the digits wider; step the type down to fit.
        let crowded = self.state.prefs.seconds || !suffix.is_empty();
        let time_px = if crowded { 42.0 } else { 54.0 } * k;
        let top = if self.state.prefs.date { 18.0 } else { 16.0 } * k;
        let (time_card, time_w, _) = ui::text(
            c,
            gfx,
            &time,
            &TextStyle { px: time_px, weight: 300, display: true, ..Default::default() },
        )?;
        time_card.SetOffset(v3(24.0 * k, top, 0.0))?;
        root.Children()?.InsertAtTop(&time_card)?;

        if !suffix.is_empty() {
            let (suffix_card, _, _) = ui::text(
                c,
                gfx,
                &suffix,
                &TextStyle { px: 16.0 * k, weight: 500, color: theme::text_2(), ..Default::default() },
            )?;
            suffix_card.SetOffset(v3(24.0 * k + time_w + 8.0 * k, top + 26.0 * k, 0.0))?;
            root.Children()?.InsertAtTop(&suffix_card)?;
        }

        if self.state.prefs.date {
            // A hairline under the time, like a printed card.
            let (rule, _, _) = ui::rounded(c, card_w - 48.0 * k, 1.0 * k, 0.0, theme::ink(0.12))?;
            rule.SetOffset(v3(24.0 * k, 82.0 * k, 0.0))?;
            root.Children()?.InsertAtTop(&rule)?;

            let (date_card, _, _) = ui::text(
                c,
                gfx,
                &date,
                &TextStyle { px: 14.0 * k, weight: 500, color: theme::text_2(), ..Default::default() },
            )?;
            date_card.SetOffset(v3(26.0 * k, 90.0 * k, 0.0))?;
            root.Children()?.InsertAtTop(&date_card)?;
        }

        self.state.drawn = key;
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

impl Drop for DesktopClock {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// The card's size in physical pixels for these settings.
fn card_size(prefs: Prefs, k: f32) -> (f32, f32) {
    let h = if prefs.date { CARD_H } else { CARD_H_SHORT };
    (CARD_W * k, h * k)
}

fn local_time() -> SYSTEMTIME {
    unsafe { GetLocalTime() }
}

fn state_of(hwnd: HWND) -> Option<&'static mut ClockState> {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ClockState;
    unsafe { raw.as_mut() }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // Right-click (and the keyboard menu key) opens the card's own menu.
        WM_RBUTTONUP | WM_CONTEXTMENU => {
            if let Some(state) = state_of(hwnd) {
                let _ = unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(state.dock),
                        crate::dock::WM_APP_CLOCK_MENU,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Some(state) = state_of(hwnd) {
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
                    // Keep the card on the display it belongs to while it is dragged.
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
                    // Snap to the nearest corner and let the panel remember it.
                    let corner = state.nearest_corner();
                    state.prefs.corner = corner;
                    state.place();
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                            Some(state.dock),
                            crate::dock::WM_APP_CLOCK_MOVED,
                            WPARAM(corner.code()),
                            LPARAM(0),
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
        assert!(CARD_W / CARD_H < 3.0);
    }

    #[test]
    fn russian_names_cover_every_day_and_month() {
        assert_eq!(WEEKDAYS.len(), 7);
        assert_eq!(MONTHS.len(), 12);
        assert!(MONTHS.iter().all(|month| month.chars().all(|ch| ch.is_alphabetic())));
    }

    #[test]
    fn every_corner_is_inside_the_monitor() {
        let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        for corner in ClockCorner::ALL {
            let (x, y) = corner_origin(corner, monitor, 268, 132, 24);
            assert!(x >= monitor.left && x + 268 <= monitor.right, "{corner:?} leaves the screen sideways");
            assert!(y >= monitor.top && y + 132 <= monitor.bottom, "{corner:?} leaves the screen vertically");
        }
    }

    #[test]
    fn the_four_corners_are_four_places() {
        let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        let places: std::collections::BTreeSet<(i32, i32)> = ClockCorner::ALL
            .iter()
            .map(|corner| corner_origin(*corner, monitor, 268, 132, 24))
            .collect();
        assert_eq!(places.len(), 4);
    }

    #[test]
    fn a_bigger_card_still_fits_the_smallest_display() {
        let monitor = RECT { left: 0, top: 0, right: 1024, bottom: 768 };
        let k = 1.0 * 1.5;
        let (w, h) = card_size(Prefs { corner: ClockCorner::BottomRight, scale: 1.5, hour12: false, date: true, seconds: false }, k);
        let (x, y) = corner_origin(ClockCorner::BottomRight, monitor, w as i32, h as i32, 36);
        assert!(x >= monitor.left && y >= monitor.top);
    }

    #[test]
    fn twelve_hour_time_has_no_leading_zero_and_a_suffix() {
        let prefs = |hour12, seconds| Prefs { corner: ClockCorner::TopRight, scale: 1.0, hour12, date: true, seconds };
        let now = SYSTEMTIME {
            wYear: 2026,
            wMonth: 11,
            wDay: 19,
            wHour: 15,
            wMinute: 7,
            wSecond: 4,
            ..Default::default()
        };
        let (time, suffix) = time_text(prefs(true, false), &now);
        assert_eq!(time, "3:07");
        assert_eq!(suffix, "PM");
        let (with_seconds, _) = time_text(prefs(true, true), &now);
        assert_eq!(with_seconds, "3:07:04");
        let (twenty_four, empty) = time_text(prefs(false, false), &now);
        assert_eq!(twenty_four, "15:07");
        assert!(empty.is_empty());
        assert_eq!(date_text(&now), "четверг, 19 ноября");
    }
}
