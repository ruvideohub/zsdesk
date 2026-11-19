//! The "sound and status" card that opens above the dock.
//!
//! A layered popup, like the dock itself: per-pixel alpha, no activation, its
//! own 16 ms timer while it is open. It takes mouse capture so a click anywhere
//! else closes it without stealing focus from the foreground application.
//!
//! Everything that can be decided without a window (geometry, slider mapping,
//! click targets) lives in [`crate::panel`].

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::{GetDC, ReleaseDC, AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION},
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, SetCapture, VK_ESCAPE},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, KillTimer,
                SetTimer, SetWindowPos, ShowWindow, UpdateLayeredWindow,
                SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_DESTROY,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_TIMER, WS_EX_LAYERED,
                WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
            },
        },
    },
};

use zsdesk_windows::gfx::{Align, Canvas, Color, Gfx, RectF, Surface, TextStyle};

use crate::{
    apps,
    layout::Rect,
    panel::{self, Hit},
    tray,
};

/// Window class of the card.
pub const CLASS: &str = "ZSDesk.Dock.Panel";
const TIMER_TICK: usize = 3;
/// How often the card re-reads the volume while it is open.
const REFRESH: Duration = Duration::from_millis(250);

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

/// One open card.
pub struct Panel {
    hwnd: HWND,
    gfx: Gfx,
    surface: Option<Surface>,
    area: RECT,
    anchor: RECT,
    snapshot: tray::Snapshot,
    hover: Option<Hit>,
    dragging: bool,
    dark: bool,
    accent: [u8; 3],
    dpi: u32,
    last_refresh: Instant,
    dirty: bool,
}

/// Colour set of the card, following the system light/dark preference.
struct Palette {
    card: Color,
    border: Color,
    text: Color,
    secondary: Color,
    plate: Color,
    accent: Color,
    shadow: Color,
}

impl Palette {
    fn for_mode(dark: bool, accent: [u8; 3]) -> Self {
        if dark {
            Self {
                card: Color::rgba8(28, 31, 40, 214),
                border: Color::rgba8(255, 255, 255, 38),
                text: Color::rgba8(240, 243, 249, 255),
                secondary: Color::rgba8(240, 243, 249, 150),
                plate: Color::rgba8(255, 255, 255, 24),
                accent: Color::rgba8(accent[0], accent[1], accent[2], 255),
                shadow: Color::rgba8(0, 0, 0, 96),
            }
        } else {
            Self {
                card: Color::rgba8(250, 250, 252, 226),
                border: Color::rgba8(0, 0, 0, 26),
                text: Color::rgba8(22, 24, 30, 255),
                secondary: Color::rgba8(22, 24, 30, 140),
                plate: Color::rgba8(0, 0, 0, 14),
                accent: Color::rgba8(accent[0], accent[1], accent[2], 255),
                shadow: Color::rgba8(0, 0, 0, 54),
            }
        }
    }
}

/// Opens the card anchored above the status indicator at `anchor` (screen
/// coordinates), clamped to the monitor work area `area`.
pub fn open(anchor: RECT, area: RECT, dark: bool, accent: [u8; 3]) -> windows::core::Result<()> {
    let family = zsdesk_windows::gfx::system_ui_font(None).0;
    let gfx = Gfx::new(&family).map_err(|error| {
        windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), error.message())
    })?;
    let probe = panel::layout(1.0);
    let class: Vec<u16> = CLASS.encode_utf16().chain(Some(0)).collect();
    let title = zsdesk_core::i18n::wide("声音");
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
        .map(|module| windows::Win32::Foundation::HINSTANCE(module.0))?;
    let hwnd = unsafe {
        CreateWindowExW(WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()), WS_POPUP, area.left, area.top,
            probe.card.w as i32, probe.card.h as i32, None, None, Some(instance), None)
    }?;
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let mut panel = Panel {
        hwnd,
        gfx,
        surface: None,
        area,
        anchor,
        snapshot: tray::query(),
        hover: None,
        dragging: false,
        dark,
        accent,
        dpi,
        last_refresh: Instant::now(),
        dirty: true,
    };
    panel.place();
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    unsafe { SetCapture(hwnd) };
    unsafe { let _ = SetTimer(Some(hwnd), TIMER_TICK, 16, None); }
    PANEL.with(|slot| *slot.borrow_mut() = Some(panel));
    with_panel(|panel| panel.paint());
    Ok(())
}

/// `true` while a card is on screen.
pub fn is_open() -> bool { PANEL.with(|slot| slot.borrow().is_some()) }

/// Closes the card, if any.
pub fn close() {
    let previous = PANEL.with(|slot| slot.borrow_mut().take());
    if let Some(panel) = previous {
        let _ = unsafe { ReleaseCapture() };
        let _ = unsafe { KillTimer(Some(panel.hwnd), TIMER_TICK) };
        let _ = unsafe { DestroyWindow(panel.hwnd) };
    }
}

fn with_panel(action: impl FnOnce(&mut Panel)) {
    let busy = PANEL.with(|slot| slot.try_borrow_mut().is_err());
    if busy { return; }
    PANEL.with(|slot| {
        if let Some(panel) = slot.borrow_mut().as_mut() {
            action(panel);
        }
    });
}

pub unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = PANEL.with(|slot| match slot.try_borrow_mut() {
        Ok(mut slot) => slot.as_mut().and_then(|panel| panel.on_message(hwnd, message, wparam, lparam)),
        Err(_) => None,
    });
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) })
}

impl Panel {
    fn factor(&self) -> f32 { (self.dpi.max(96) as f32) / 96.0 }

    /// Places the card above the anchor, kept inside the work area.
    fn place(&mut self) {
        let layout = panel::layout(self.factor());
        let width = layout.card.w.round() as i32;
        let height = layout.card.h.round() as i32;
        let gap = (12.0 * self.factor()).round() as i32;
        let max_x = (self.area.right - width - gap).max(self.area.left + gap);
        let x = (self.anchor.right - width).clamp(self.area.left + gap, max_x);
        let y = (self.anchor.top - height - gap).max(self.area.top + gap);
        let _ = unsafe { SetWindowPos(self.hwnd, None, x, y.max(0), width, height,
            SWP_NOZORDER | SWP_NOACTIVATE) };
    }

    fn on_message(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        let layout = panel::layout(self.factor());
        match message {
            WM_MOUSEMOVE => {
                let point = point_from_lparam(lparam);
                if self.dragging {
                    let volume = layout.volume_at(point.x as f32);
                    if self.set_volume(volume) { self.dirty = true; }
                }
                let hover = layout.hit(point.x as f32, point.y as f32);
                if hover != self.hover {
                    self.hover = hover;
                    self.dirty = true;
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let point = point_from_lparam(lparam);
                match layout.hit(point.x as f32, point.y as f32) {
                    Some(Hit::Track) => {
                        self.dragging = true;
                        let volume = layout.volume_at(point.x as f32);
                        if self.set_volume(volume) { self.dirty = true; }
                    }
                    Some(_) => {}
                    // A click on the desktop or on the dock closes the card.
                    None => close(),
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let point = point_from_lparam(lparam);
                self.dragging = false;
                if let Some(hit) = layout.hit(point.x as f32, point.y as f32) {
                    self.activate(hit);
                } else {
                    close();
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == TIMER_TICK => {
                if unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) } < 0 {
                    close();
                    return Some(LRESULT(0));
                }
                // The pointer may leave the card without a message when the
                // capture is lost; follow it instead of trusting the last move.
                if let Ok(point) = cursor_in(self.hwnd) {
                    let hover = layout.hit(point.x as f32, point.y as f32);
                    if hover != self.hover { self.hover = hover; self.dirty = true; }
                }
                if self.last_refresh.elapsed() >= REFRESH {
                    self.last_refresh = Instant::now();
                    let sample = tray::query();
                    if sample.volume != self.snapshot.volume || sample.muted != self.snapshot.muted
                        || sample.online != self.snapshot.online || sample.battery != self.snapshot.battery
                        || sample.charging != self.snapshot.charging {
                        self.snapshot = sample;
                        self.dirty = true;
                    }
                }
                if self.dirty {
                    self.dirty = false;
                    self.paint();
                }
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                let _ = unsafe { KillTimer(Some(hwnd), TIMER_TICK) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn set_volume(&mut self, volume: u8) -> bool {
        if self.snapshot.volume == Some(volume) && !self.snapshot.muted {
            return false;
        }
        if !tray::set_volume(volume) {
            return false;
        }
        self.snapshot.volume = Some(volume);
        self.snapshot.muted = false;
        true
    }

    fn activate(&mut self, hit: Hit) {
        match hit {
            Hit::Mute => {
                let muted = !self.snapshot.muted;
                if tray::set_muted(muted) {
                    self.snapshot.muted = muted;
                    self.dirty = true;
                }
            }
            Hit::Track => {}
            Hit::Row(0) => { let _ = apps::launch("ms-availablenetworks:"); close(); }
            Hit::Row(1) => { let _ = apps::launch("ms-settings:batterysaver-settings"); close(); }
            Hit::Chip(0) => { tray::quick_settings(); close(); }
            Hit::Chip(1) => { tray::notifications(); close(); }
            Hit::Chip(2) => { let _ = apps::launch("ms-settings:sound"); close(); }
            Hit::Chip(_) => { let _ = apps::launch("ms-settings:network"); close(); }
            Hit::Row(_) => {}
        }
    }

    fn paint(&mut self) {
        let factor = self.factor();
        let layout = panel::layout(factor);
        let width = layout.card.w.round() as i32;
        let height = layout.card.h.round() as i32;
        if self.surface.as_ref().is_none_or(|surface| surface.width != width || surface.height != height) {
            self.surface = Surface::new(width, height);
        }
        let palette = Palette::for_mode(self.dark, self.accent);
        let snapshot = self.snapshot;
        let hover = self.hover;
        let radius = 18.0 * factor;
        let caption = zsdesk_core::i18n::text("声音").to_string();
        let value = match (snapshot.muted, snapshot.volume) {
            (true, _) => zsdesk_core::i18n::text("静音").to_string(),
            (false, Some(level)) => format!("{level}%"),
            (false, None) => String::new(),
        };
        let rows: Vec<(String, String)> = vec![
            (zsdesk_core::i18n::text("网络").to_string(),
                zsdesk_core::i18n::text(if snapshot.online { "已连接" } else { "未连接" }).to_string()),
            (zsdesk_core::i18n::text("电池").to_string(), battery_text(snapshot)),
        ];
        let chips: Vec<String> = vec![
            zsdesk_core::i18n::text("快速设置").to_string(),
            zsdesk_core::i18n::text("通知中心").to_string(),
            zsdesk_core::i18n::text("声音设置").to_string(),
            zsdesk_core::i18n::text("网络设置").to_string(),
        ];
        let caption_style = TextStyle::ui(12.5 * factor).weight(600);
        let value_style = TextStyle::ui(11.5 * factor).align(Align::End);
        let row_style = TextStyle::ui(12.0 * factor);
        let row_value_style = TextStyle::ui(11.5 * factor).align(Align::End);
        let chip_style = TextStyle::ui(11.5 * factor);
        let knob = layout.knob_x(snapshot.volume.unwrap_or(0));
        let muted = snapshot.muted;
        let Some(surface) = self.surface.as_ref() else { return };
        let gfx = &self.gfx;
        let drawn = gfx.draw(surface, |canvas| {
            // Soft shadow keeps the card readable on bright wallpapers.
            for step in 1..=3 {
                let grow = step as f32 * 2.0;
                canvas.round(RectF::new(-grow, -grow + 3.0, width as f32 + grow * 2.0, height as f32 + grow * 2.0),
                    radius + grow, palette.shadow.with_alpha(0.6 / step as f32));
            }
            canvas.round(rf(layout.card), radius, palette.card);
            canvas.stroke_round(rf(layout.card), radius, palette.border, 1.0);

            canvas.text(&caption, rf(layout.header), &caption_style, palette.text);
            canvas.text(&value, rf(layout.value), &value_style, palette.secondary);

            if hover == Some(Hit::Mute) {
                canvas.round(rf(layout.mute), layout.mute.h * 0.32, palette.plate);
            }
            let glyph = layout.mute.h * 0.62;
            canvas.glyph(if muted { "\u{E74F}" } else { "\u{E767}" },
                rect_centre(layout.mute, glyph), glyph, if muted { palette.secondary } else { palette.text });

            // Slider: rail, filled part, knob.
            let rail_height = 5.0 * factor;
            let rail = RectF::new(layout.track.x, layout.track.y + (layout.track.h - rail_height) / 2.0,
                layout.track.w, rail_height);
            canvas.round(rail, rail_height / 2.0, palette.secondary.with_alpha(0.34));
            if let Some(level) = snapshot.volume {
                if !muted {
                    let filled = RectF::new(rail.x, rail.y, (level as f32 / 100.0).max(0.0) * rail.w, rail.h);
                    canvas.round(filled, rail_height / 2.0, palette.accent);
                }
            }
            if hover == Some(Hit::Track) || self.dragging {
                canvas.round(RectF::new(rail.x + knob - 7.0 * factor, rail.y - 7.0 * factor,
                    14.0 * factor, 14.0 * factor), 7.0 * factor, palette.accent.with_alpha(0.28));
            }
            let knob_size = 13.0 * factor;
            canvas.round(RectF::new(rail.x + knob - knob_size / 2.0, rail.y + rail.h / 2.0 - knob_size / 2.0,
                knob_size, knob_size), knob_size / 2.0, palette.text.with_alpha(0.95));

            // Separator between the slider and the status rows.
            let separator_y = layout.rows[0].y - (5.0 * factor);
            canvas.fill(RectF::new(layout.card.x + 16.0 * factor, separator_y,
                layout.card.w - 32.0 * factor, 1.0), palette.border.with_alpha(1.4));

            for (index, (label, right)) in rows.iter().enumerate() {
                let row = layout.rows[index];
                if hover == Some(Hit::Row(index)) {
                    canvas.round(RectF::new(row.x - 6.0 * factor, row.y, row.w + 12.0 * factor, row.h),
                        8.0 * factor, palette.plate);
                }
                let glyph_size = 15.0 * factor;
                let icon = RectF::new(row.x + 2.0 * factor, row.y + (row.h - glyph_size) / 2.0, glyph_size, glyph_size);
                match index {
                    0 => canvas.glyph("\u{E701}", icon, glyph_size,
                        if snapshot.online { palette.text } else { palette.secondary }),
                    _ => draw_battery(canvas, icon, snapshot, palette.text, palette.secondary, palette.accent),
                }
                let text_left = icon.right() + 9.0 * factor;
                canvas.text(label, RectF::new(text_left, row.y, row.w * 0.55, row.h), &row_style, palette.text);
                canvas.text(right, RectF::new(row.x, row.y, row.w - 2.0 * factor, row.h),
                    &row_value_style, palette.secondary);
            }

            for (index, label) in chips.iter().enumerate() {
                let chip = layout.chips[index];
                let hot = hover == Some(Hit::Chip(index));
                canvas.round(rf(chip), 10.0 * factor,
                    if hot { palette.plate.with_alpha(2.6) } else { palette.plate });
                canvas.stroke_round(rf(chip), 10.0 * factor, palette.border.with_alpha(0.7), 1.0);
                let glyph = match index {
                    0 => "\u{E713}",
                    1 => "\u{EA8F}",
                    2 => "\u{E767}",
                    _ => "\u{E701}",
                };
                let glyph_size = 14.0 * factor;
                let icon = RectF::new(chip.x + 10.0 * factor, chip.y + (chip.h - glyph_size) / 2.0, glyph_size, glyph_size);
                canvas.glyph(glyph, icon, glyph_size, palette.secondary);
                canvas.text(label, RectF::new(icon.right() + 8.0 * factor, chip.y,
                    chip.w - (icon.right() - chip.x) - 12.0 * factor, chip.h), &chip_style, palette.text);
            }
        });
        if drawn.is_err() { return; }
        let Some(surface) = self.surface.as_ref() else { return };
        let screen = unsafe { GetDC(None) };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8 };
        let origin = POINT::default();
        // The window position is managed by `place`; only the frame is pushed.
        let position = window_origin(self.hwnd);
        let size = SIZE { cx: width, cy: height };
        let _ = unsafe { UpdateLayeredWindow(self.hwnd, Some(screen), Some(&position), Some(&size),
            Some(surface.dc), Some(&origin), Default::default(), Some(&blend), ULW_ALPHA) };
        unsafe { ReleaseDC(None, screen) };
    }
}

/// `"76% · зарядка"` or just `"—"` when the machine has no battery.
fn battery_text(snapshot: tray::Snapshot) -> String {
    let Some(level) = snapshot.battery else { return String::new() };
    if snapshot.charging {
        format!("{level}% · {}", zsdesk_core::i18n::text("正在充电"))
    } else {
        format!("{level}%")
    }
}

/// Paints the battery the same way the dock does, reusing the look.
fn draw_battery(canvas: &Canvas, rect: RectF, snapshot: tray::Snapshot,
    text: Color, dim: Color, accent: Color) {
    let width = rect.w * 0.95;
    let height = rect.h * 0.5;
    let x = rect.x;
    let y = rect.y + (rect.h - height) / 2.0;
    let stroke = if snapshot.battery.is_some() { text } else { dim };
    canvas.stroke_round(RectF::new(x, y, width, height), height * 0.32, stroke.with_alpha(0.75), 1.2);
    canvas.round(RectF::new(x + width + 1.2, y + height * 0.3, 1.6, height * 0.4), 0.8, stroke.with_alpha(0.6));
    if let Some(level) = snapshot.battery {
        let inner = ((width - 3.4) * (level as f32 / 100.0)).max(1.4);
        let low = level <= 20 && !snapshot.charging;
        canvas.round(RectF::new(x + 1.7, y + 1.7, inner, height - 3.4), (height - 3.4) * 0.3,
            if low { Color::rgba8(255, 159, 107, 255) } else { text });
        if snapshot.charging {
            canvas.round(RectF::new(x + width / 2.0 - 1.0, y - height * 0.5, 2.0, height * 0.32), 1.0, accent);
        }
    }
}

fn cursor_in(hwnd: HWND) -> windows::core::Result<POINT> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }?;
    if !unsafe { windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point) }.as_bool() {
        return Ok(POINT::default());
    }
    Ok(point)
}

fn window_origin(hwnd: HWND) -> POINT {
    let mut rect = RECT::default();
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) };
    POINT { x: rect.left, y: rect.top }
}

fn rect_centre(rect: Rect, size: f32) -> RectF {
    RectF::new(rect.x + (rect.w - size) / 2.0, rect.y + (rect.h - size) / 2.0, size, size)
}

fn rf(rect: Rect) -> RectF { RectF::new(rect.x, rect.y, rect.w, rect.h) }

fn point_from_lparam(lparam: LPARAM) -> POINT {
    POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 }
}
