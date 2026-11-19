//! The desktop clock: a quiet glass card in the corner of the desktop, macOS style.
//!
//! It runs in the panel's process but not in the panel's window. It is an ordinary,
//! *not topmost*, click-through window, so it sits above the wallpaper and below
//! application windows — the way desktop widgets behave on a Mac — and it never takes a
//! click. The time refreshes once a second; only a changed minute redraws anything.

use windows::core::{w, Result};
use windows::UI::Composition::*;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_EXCLUDED_FROM_PEEK, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::System::Time::{GetLocalTime, SYSTEMTIME};
use windows::Win32::System::WinRT::Composition::{ICompositorDesktopInterop, DesktopWindowTarget};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SetWindowPos, ShowWindow,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::gfx::{Gfx, TextStyle};
use crate::ui::{self, theme, v3};

/// The card, in device-independent pixels.
const CARD_W: f32 = 268.0;
const CARD_H: f32 = 132.0;
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

pub struct DesktopClock {
    hwnd: HWND,
    root: ContainerVisual,
    _target: DesktopWindowTarget,
    scale: f32,
    /// The last minute drawn, so a tick that changes nothing costs nothing.
    drawn: String,
}

impl DesktopClock {
    /// Create the card on `monitor`, or return an error the caller can simply ignore:
    /// a desktop without a clock is fine, a desktop that fails to start is not.
    pub fn new(c: &Compositor, monitor: RECT, scale: f32) -> Result<Self> {
        unsafe {
            let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassW(&class);
            // No redirection bitmap (Composition draws), no activation, no hit testing.
            let ex = WS_EX_NOREDIRECTIONBITMAP
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE
                | WS_EX_TRANSPARENT;
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
            let clock = Self { hwnd, root, _target: target, scale, drawn: String::new() };
            clock.place(monitor, scale);
            Ok(clock)
        }
    }

    fn place(&self, monitor: RECT, scale: f32) {
        let s = scale.max(1.0);
        let (w, h) = ((CARD_W * s) as i32, (CARD_H * s) as i32);
        let margin = (MARGIN * s) as i32;
        let x = monitor.right - w - margin;
        let y = monitor.top + margin;
        unsafe {
            // Not HWND_TOPMOST: the card belongs to the desktop, not above your work.
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Draw the card. Called on a one-second timer; redraws only when the minute changed.
    pub fn tick(&mut self, c: &Compositor, gfx: &Gfx, scale: f32) -> Result<()> {
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            self.drawn.clear();
        }
        let now = local_time();
        let time = format!("{:02}:{:02}", now.wHour, now.wMinute);
        if time == self.drawn {
            return Ok(());
        }
        let date = format!(
            "{}, {} {}",
            WEEKDAYS[(now.wDayOfWeek as usize).min(6)],
            now.wDay,
            MONTHS[(now.wMonth.saturating_sub(1) as usize).min(11)]
        );

        let s = self.scale.max(1.0);
        let root = &self.root;
        root.Children()?.RemoveAll()?;

        // Glass, the same surface the panel and Launchpad use.
        let (bg, _, _) = ui::rounded(c, CARD_W * s, CARD_H * s, 26.0 * s, theme::surface(0.55))?;
        root.Children()?.InsertAtTop(&bg)?;

        let (time_card, _, _) = ui::text(
            c,
            gfx,
            &time,
            &TextStyle { px: 54.0 * s, weight: 300, display: true, ..Default::default() },
        )?;
        time_card.SetOffset(v3(24.0 * s, 20.0 * s, 0.0))?;
        root.Children()?.InsertAtTop(&time_card)?;

        let (date_card, _, _) = ui::text(
            c,
            gfx,
            &date,
            &TextStyle { px: 14.0 * s, weight: 500, color: theme::text_2(), ..Default::default() },
        )?;
        date_card.SetOffset(v3(26.0 * s, 88.0 * s, 0.0))?;
        root.Children()?.InsertAtTop(&date_card)?;

        // A hairline under the time, like a printed card.
        let (rule, _, _) = ui::rounded(c, (CARD_W - 48.0) * s, 1.0 * s, 0.0, theme::ink(0.12))?;
        rule.SetOffset(v3(24.0 * s, 82.0 * s, 0.0))?;
        root.Children()?.InsertAtTop(&rule)?;

        self.drawn = time;
        Ok(())
    }

    pub fn destroy(&mut self) {
        if !self.hwnd.0.is_null() {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
            self.hwnd = HWND(std::ptr::null_mut());
        }
    }
}

impl Drop for DesktopClock {
    fn drop(&mut self) {
        self.destroy();
    }
}

fn local_time() -> SYSTEMTIME {
    let mut now = SYSTEMTIME::default();
    unsafe {
        GetLocalTime(&mut now);
    }
    now
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    DefWindowProcW(hwnd, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    /// The card must never be taller than it is wide, or the corner layout stops looking
    /// like a macOS widget.
    #[test]
    fn the_card_keeps_a_widget_shape() {
        assert!(super::CARD_W > super::CARD_H);
        assert!(super::CARD_W / super::CARD_H < 3.0);
    }

    #[test]
    fn russian_names_cover_every_day_and_month() {
        assert_eq!(super::WEEKDAYS.len(), 7);
        assert_eq!(super::MONTHS.len(), 12);
        assert!(super::MONTHS.iter().all(|month| month.chars().all(|ch| ch.is_alphabetic())));
    }
}
