//! The "all applications" overlay: a full-screen translucent grid with a
//! search field, opened from the dock's grid icon.
//!
//! The window is layered like the dock, but it is *not* `WS_EX_NOACTIVATE`:
//! it takes the keyboard so typing filters the grid, Enter starts the first
//! match and Escape closes it.

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
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, KillTimer, PostQuitMessage,
                SetForegroundWindow, SetTimer, ShowWindow, UpdateLayeredWindow, WM_APP,
                WM_CHAR, WM_DESTROY, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
                WM_TIMER, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP, SW_SHOW, ULW_ALPHA,
            },
        },
    },
};

use zsdesk_windows::gfx::{Align, Color, Gfx, RectF, Surface, TextStyle};

use crate::{
    apps::{self, AppEntry},
    icons::{IconCache, IconLoader},
    layout::Rect,
};

pub const CLASS: &str = "ZSDesk.Dock.Launcher";
const WM_ICON_READY: u32 = WM_APP + 502;
const TIMER_TICK: usize = 2;

thread_local! {
    static LAUNCHER: RefCell<Option<Launcher>> = const { RefCell::new(None) };
}

/// One open overlay.
pub struct Launcher {
    hwnd: HWND,
    gfx: Gfx,
    surface: Option<Surface>,
    entries: Vec<AppEntry>,
    matches: Vec<usize>,
    query: String,
    selection: usize,
    scroll: usize,
    area: RECT,
    icons: IconCache,
    loader: IconLoader,
    dark: bool,
    accent: [u8; 3],
    icon_size: f32,
    dpi: u32,
    last_input: Instant,
    dirty: bool,
}

/// Creates and shows the overlay; the caller installs it with [`install`].
pub fn open(area: RECT, entries: Vec<AppEntry>, icon_size: f32, dark: bool, accent: [u8; 3])
    -> windows::core::Result<Launcher>
{
    let family = zsdesk_windows::gfx::system_ui_font(None).0;
    let gfx = Gfx::new(&family).map_err(|error| {
        windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), error.message())
    })?;
    let width = (area.right - area.left).max(320);
    let height = (area.bottom - area.top).max(240);
    let class: Vec<u16> = CLASS.encode_utf16().chain(Some(0)).collect();
    let title = zsdesk_core::i18n::wide("全部应用");
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
        .map(|module| windows::Win32::Foundation::HINSTANCE(module.0))?;
    let hwnd = unsafe {
        CreateWindowExW(WS_EX_LAYERED | WS_EX_TOOLWINDOW, PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr()),
            WS_POPUP, area.left, area.top, width, height, None, None, Some(instance), None)
    }?;
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let mut launcher = Launcher {
        hwnd,
        gfx,
        surface: None,
        entries,
        matches: Vec::new(),
        query: String::new(),
        selection: 0,
        scroll: 0,
        area,
        icons: IconCache::default(),
        loader: IconLoader::spawn(hwnd, WM_ICON_READY),
        dark,
        accent,
        icon_size: icon_size.max(32.0),
        dpi,
        last_input: Instant::now(),
        dirty: true,
    };
    launcher.refilter();
    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
    let _ = unsafe { SetForegroundWindow(hwnd) };
    unsafe { let _ = SetTimer(Some(hwnd), TIMER_TICK, 30, None); }
    Ok(launcher)
}

/// Makes `launcher` the overlay of this thread, replacing a previous one.
pub fn install(launcher: Launcher) {
    LAUNCHER.with(|slot| {
        if let Some(mut previous) = slot.borrow_mut().take() {
            previous.loader.shutdown();
            let _ = unsafe { DestroyWindow(previous.hwnd) };
        }
        *slot.borrow_mut() = Some(launcher);
    });
}

pub unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = LAUNCHER.with(|slot| match slot.try_borrow_mut() {
        Ok(mut slot) => slot.as_mut().and_then(|launcher| launcher.on_message(hwnd, message, wparam, lparam)),
        Err(_) => None,
    });
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) })
}

impl Launcher {
    fn on_message(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match message {
            WM_CHAR => {
                if let Some(character) = char::from_u32(wparam.0 as u32) {
                    if !character.is_control() {
                        self.query.push(character);
                        self.refilter();
                    }
                }
                self.last_input = Instant::now();
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                use windows::Win32::UI::Input::KeyboardAndMouse::{VK_BACK, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
                    VK_LEFT, VK_RETURN, VK_RIGHT, VK_UP};
                let key = wparam.0 as u16;
                match key {
                    key if key == VK_BACK.0 => { self.query.pop(); self.refilter(); }
                    key if key == VK_ESCAPE.0 => self.close(),
                    key if key == VK_RETURN.0 => self.launch_selected(),
                    key if key == VK_LEFT.0 => self.move_selection(-1),
                    key if key == VK_RIGHT.0 => self.move_selection(1),
                    key if key == VK_UP.0 => self.move_selection(-(self.metrics().0 as isize)),
                    key if key == VK_DOWN.0 => self.move_selection(self.metrics().0 as isize),
                    key if key == VK_HOME.0 => self.move_selection(isize::MIN / 2),
                    key if key == VK_END.0 => self.move_selection(isize::MAX / 2),
                    _ => {}
                }
                self.last_input = Instant::now();
                self.dirty = true;
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let point = point_from_lparam(lparam);
                if let Some(index) = self.tile_at(point) {
                    if self.selection != index { self.selection = index; self.dirty = true; }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let point = point_from_lparam(lparam);
                match self.tile_at(point) {
                    Some(index) => { self.selection = index; self.launch_selected(); }
                    // A click on the dimmed area outside the grid closes it.
                    None => if !self.grid_area().contains(point.x as f32, point.y as f32) { self.close() },
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
                let (columns, rows) = self.metrics();
                let (scroll, pages) = crate::layout::clamp_page(self.scroll, columns, rows, self.matches.len());
                self.scroll = if delta < 0 { (scroll + 1).min(pages - 1) } else { scroll.saturating_sub(1) };
                self.dirty = true;
                Some(LRESULT(0))
            }
            WM_KILLFOCUS => { self.close(); Some(LRESULT(0)) }
            WM_ICON_READY => { self.loader.drain(&self.gfx, &mut self.icons); self.dirty = true; Some(LRESULT(0)) }
            WM_TIMER if wparam.0 == TIMER_TICK => {
                self.loader.drain(&self.gfx, &mut self.icons);
                // The caret blinks: repaint twice a second.
                if self.last_input.elapsed() > Duration::from_millis(400) { self.dirty = true; self.last_input = Instant::now(); }
                if self.dirty { self.dirty = false; self.paint(); }
                Some(LRESULT(0))
            }
            WM_DESTROY => { let _ = unsafe { KillTimer(Some(hwnd), TIMER_TICK) }; unsafe { PostQuitMessage(0) }; Some(LRESULT(0)) }
            _ => None,
        }
    }

    fn close(&mut self) {
        self.loader.shutdown();
        let _ = unsafe { DestroyWindow(self.hwnd) };
        LAUNCHER.with(|slot| *slot.borrow_mut() = None);
    }

    fn refilter(&mut self) {
        self.matches = crate::catalogue::filter(&self.entries, &self.query);
        self.selection = 0;
        self.scroll = 0;
        self.dirty = true;
        self.request_icons();
    }

    fn request_icons(&mut self) {
        let pixels = (self.icon_size * 1.4).round() as i32;
        let (columns, rows) = self.metrics();
        let mut requests = Vec::new();
        for (position, index) in self.matches.iter().enumerate().skip(self.scroll * crate::layout::page_size(columns, rows))
            .take(crate::layout::page_size(columns, rows)) {
            let _ = position;
            let entry = &self.entries[*index];
            if self.icons.want(&entry.path) { requests.push((entry.path.clone(), entry.path.clone())); }
        }
        for (key, path) in requests { self.loader.request(&key, &path, pixels.clamp(32, 256)); }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.matches.is_empty() { return; }
        let (columns, rows) = self.metrics();
        let current = (self.selection as isize).saturating_add(delta);
        self.selection = current.clamp(0, self.matches.len() as isize - 1) as usize;
        self.scroll = crate::layout::page_of(self.selection, columns, rows);
    }

    fn launch_selected(&mut self) {
        let Some(index) = self.matches.get(self.selection).copied() else { return };
        let Some(entry) = self.entries.get(index) else { return };
        let _ = apps::launch(&entry.path);
        self.close();
    }

    /// Columns and rows this screen can show at the current icon size.
    fn metrics(&self) -> (usize, usize) {
        crate::layout::grid_metrics(self.grid_area(), self.icon_size, self.dpi.max(96) as f32 / 96.0)
    }

    /// Client-space rectangle of the tile grid.
    fn grid_area(&self) -> Rect {
        let factor = self.dpi.max(96) as f32 / 96.0;
        let width = (self.area.right - self.area.left) as f32;
        let margin = 60.0 * factor;
        Rect::new(margin, 150.0 * factor, (width - margin * 2.0).max(200.0),
            ((self.area.bottom - self.area.top) as f32 - 200.0 * factor).max(120.0))
    }

    fn tile_size(&self, columns: usize, rows: usize) -> (f32, f32) {
        let grid = self.grid_area();
        (grid.w / columns.max(1) as f32, grid.h / rows.max(1) as f32)
    }

    fn tile_at(&self, point: POINT) -> Option<usize> {
        let grid = self.grid_area();
        if !grid.contains(point.x as f32, point.y as f32) { return None; }
        let (columns, rows) = self.metrics();
        let (tile_w, tile_h) = self.tile_size(columns, rows);
        let column = ((point.x as f32 - grid.x) / tile_w) as usize;
        let row = ((point.y as f32 - grid.y) / tile_h) as usize;
        if column >= columns || row >= rows { return None; }
        let position = self.scroll * crate::layout::page_size(columns, rows) + row * columns + column;
        (position < self.matches.len()).then_some(position)
    }

    fn paint(&mut self) {
        let width = (self.area.right - self.area.left).max(1);
        let height = (self.area.bottom - self.area.top).max(1);
        if self.surface.as_ref().is_none_or(|surface| surface.width != width || surface.height != height) {
            self.surface = Surface::new(width, height);
        }
        let Some(surface) = self.surface.as_ref() else { return };
        let factor = self.dpi.max(96) as f32 / 96.0;
        let dark = self.dark;
        let accent = self.accent;
        let scrim = if dark { Color::rgba8(8, 8, 10, 168) } else { Color::rgba8(240, 240, 244, 168) };
        let card = if dark { Color::rgba8(28, 28, 32, 232) } else { Color::rgba8(255, 255, 255, 238) };
        let text = if dark { Color::rgba8(244, 244, 246, 255) } else { Color::rgba8(24, 24, 28, 255) };
        let secondary = text.with_alpha(0.55);
        let selection = Color::rgba8(accent[0], accent[1], accent[2], 56);
        let grid = self.grid_area();
        let (columns, rows) = self.metrics();
        let (tile_w, tile_h) = self.tile_size(columns, rows);
        let page = crate::layout::page_size(columns, rows);
        let (scroll, pages) = crate::layout::clamp_page(self.scroll, columns, rows, self.matches.len());
        let query = self.query.clone();
        let empty = zsdesk_core::i18n::text("没有匹配的应用").to_string();
        let placeholder = zsdesk_core::i18n::text("搜索应用").to_string();
        let scale = self.icon_size * factor;
        let tiles: Vec<(usize, Rect, String, String)> = self.matches.iter().enumerate()
            .skip(scroll * page).take(page)
            .filter_map(|(position, index)| {
                let entry = self.entries.get(*index)?;
                let slot = position - scroll * page;
                let column = (slot % columns) as f32;
                let row = (slot / columns) as f32;
                Some((position, Rect::new(grid.x + column * tile_w, grid.y + row * tile_h, tile_w, tile_h),
                    entry.name.clone(), entry.path.clone()))
            })
            .collect();
        let icon_cache = &self.icons;
        let gfx = &self.gfx;
        let selection_index = self.selection;
        let icon_edge = (scale * 1.6).min(tile_h - 34.0 * factor);
        let label_style = TextStyle::ui(12.5 * factor).align(Align::Center).top().wrapped();
        let drawn = gfx.draw(surface, |canvas| {
            canvas.fill(RectF::new(0.0, 0.0, width as f32, height as f32), scrim);
            let field_width = 420.0 * factor;
            let field = RectF::new((width as f32 - field_width) / 2.0, 60.0 * factor, field_width, 40.0 * factor);
            canvas.round(field, field.h / 2.0, card);
            let shown = if query.is_empty() { placeholder.as_str() } else { query.as_str() };
            let color = if query.is_empty() { secondary } else { text };
            canvas.text(shown, RectF::new(field.x + 18.0, field.y, field.w - 36.0, field.h),
                &TextStyle::ui(15.0 * factor), color);
            if tiles.is_empty() {
                canvas.text(&empty, RectF::new(0.0, (height as f32) / 2.0 - 12.0 * factor, width as f32,
                    24.0 * factor), &TextStyle::ui(15.0 * factor).align(Align::Center), secondary);
            }
            if pages > 1 {
                // One quiet line instead of paging controls: it only appears
                // when the catalogue does not fit on a single screen.
                let counter = format!("{} / {}", scroll + 1, pages);
                canvas.text(&counter, RectF::new(grid.x, grid.bottom() + 10.0 * factor, grid.w, 20.0 * factor),
                    &TextStyle::ui(12.0 * factor).align(Align::Center), secondary);
            }
            for (position, tile, name, path) in &tiles {
                if selection_index == *position {
                    canvas.round(RectF::new(tile.x + 2.0, tile.y + 2.0, tile.w - 4.0, tile.h - 4.0),
                        12.0 * factor, selection);
                }
                let icon_box = RectF::new(tile.x + (tile.w - icon_edge) / 2.0, tile.y + 12.0 * factor,
                    icon_edge, icon_edge);
                match icon_cache.bitmap(path) {
                    Some(bitmap) => canvas.image(bitmap, icon_box, 1.0),
                    None => {
                        canvas.round(icon_box, icon_edge * 0.22, secondary.with_alpha(0.25));
                        let initial = name.chars().next()
                            .map(|value| value.to_uppercase().to_string()).unwrap_or_default();
                        canvas.text(&initial, icon_box,
                            &TextStyle::ui(icon_edge * 0.45).weight(600).align(Align::Center), text);
                    }
                }
                canvas.text(name, RectF::new(tile.x + 4.0, icon_box.bottom() + 6.0 * factor,
                    tile.w - 8.0, 30.0 * factor), &label_style, text);
            }
        });
        if drawn.is_err() { return; }
        let Some(surface) = self.surface.as_ref() else { return };
        let screen = unsafe { GetDC(None) };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8 };
        let position = POINT { x: self.area.left, y: self.area.top };
        let size = SIZE { cx: width, cy: height };
        let origin = POINT::default();
        let _ = unsafe { UpdateLayeredWindow(self.hwnd, Some(screen), Some(&position), Some(&size),
            Some(surface.dc), Some(&origin), Default::default(), Some(&blend), ULW_ALPHA) };
        unsafe { ReleaseDC(None, screen) };
    }
}

impl Drop for Launcher {
    fn drop(&mut self) { self.loader.shutdown(); }
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    POINT { x, y }
}
