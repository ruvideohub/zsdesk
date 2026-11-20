//! The launch settings window.
//!
//! One card per component: what it is, whether it runs right now, and the single switch that
//! says "keep it running". Plus the logon switch and a line about the service that does the
//! work. Drawn by hand with GDI — the same dark look as the rest of AuraUI, no toolkit.

use windows::core::{w, PCWSTR, Result};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetStockObject, HGDIOBJ, HFONT,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, DEFAULT_GUI_FONT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE,
    DT_VCENTER, DRAW_TEXT_FORMAT, HDC, LOGFONTW, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, GetWindowLongPtrW, LoadCursorW,
    PostQuitMessage, RegisterClassW, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, UpdateWindow,
    CREATESTRUCTW, GWLP_USERDATA, IDC_ARROW, MSG, SWP_NOMOVE, SWP_NOZORDER, SWP_NOACTIVATE, SW_SHOW, WM_CREATE, WM_DESTROY, WM_LBUTTONUP,
    WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
};

use crate::launch::{self, Component, Settings};

const CLASS: PCWSTR = w!("AuraServiceSettings");
const TITLE: &str = "AuraUI — запуск и компоненты";
const TIMER: usize = 1;

// The window's colours: the panel's dark surface, on purpose.
const BG: COLORREF = rgb(20, 20, 24);
const CARD_ON: COLORREF = rgb(32, 32, 38);
const CARD_OFF: COLORREF = rgb(25, 25, 30);
const TEXT: COLORREF = rgb(238, 238, 242);
const TEXT_2: COLORREF = rgb(164, 164, 174);
const TEXT_3: COLORREF = rgb(118, 118, 128);
const ACCENT: COLORREF = rgb(10, 132, 255);
const ACCENT_OFF: COLORREF = rgb(52, 52, 60);
const RUNNING: COLORREF = rgb(48, 209, 88);

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

/// What a click on a rectangle does.
#[derive(Clone, Copy, PartialEq)]
enum Action {
    /// Keep this component running, or stop it.
    Toggle(&'static str),
    /// Start at logon, or not.
    Autostart,
    /// Start the service itself, when it is not running.
    StartService,
}

struct State {
    settings: Settings,
    /// Clickable rectangles, rebuilt on every paint.
    hits: Vec<(RECT, Action)>,
    /// Title, label, detail — heights in points, created for the window's current dpi.
    fonts: [HFONT; 3],
    scale: f32,
}

impl State {
    fn font(&self, index: usize) -> HGDIOBJ {
        let font = self.fonts[index];
        if font.0.is_null() {
            unsafe { GetStockObject(DEFAULT_GUI_FONT) }
        } else {
            font.into()
        }
    }

    /// Re-create the fonts for a new dpi, and return the scale the layout should use.
    fn rescale(&mut self, hwnd: HWND) -> f32 {
        let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
        if (scale - self.scale).abs() < 0.01 && self.fonts.iter().all(|font| !font.0.is_null()) {
            return self.scale;
        }
        self.scale = scale;
        for font in self.fonts {
            if !font.0.is_null() {
                let _ = unsafe { DeleteObject(font.into()) };
            }
        }
        self.fonts = [make_font(scale, 17.0, 600), make_font(scale, 13.0, 600), make_font(scale, 12.5, 400)];
        scale
    }
}

fn make_font(scale: f32, size: f32, weight: i32) -> HFONT {
    let mut log = LOGFONTW { lfHeight: -(size * scale).round() as i32, lfWeight: weight, ..Default::default() };
    let family: Vec<u16> = "Segoe UI".encode_utf16().collect();
    let length = family.len().min(log.lfFaceName.len());
    log.lfFaceName[..length].copy_from_slice(&family[..length]);
    unsafe { CreateFontIndirectW(&log) }
}

/// Open the settings window; returns when it is closed.
pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: HINSTANCE(module.0),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        let state = Box::new(State {
            settings: launch::load(),
            hits: Vec::new(),
            fonts: [HFONT::default(); 3],
            scale: 1.0,
        });
        let pointer = Box::into_raw(state);
        let title: Vec<u16> = TITLE.encode_utf16().chain(Some(0)).collect();
        let hwnd = {
            let created = CreateWindowExW(
                WS_EX_APPWINDOW,
                CLASS,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                160,
                140,
                780,
                640,
                None,
                None,
                Some(HINSTANCE(module.0)),
                Some(pointer.cast()),
            );
            match created {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    drop(Box::from_raw(pointer));
                    return Err(error);
                }
            }
        };
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let _ = SetTimer(Some(hwnd), TIMER, 1000, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Ok(())
    }
}

/// The client height the content needs at this scale; the window is grown to fit it.
fn content_height(scale: f32) -> i32 {
    ((20.0 + 30.0 + 30.0 + launch::COMPONENTS.len() as f32 * 84.0 + 44.0 + 96.0 + 32.0) * scale).round() as i32
}

fn fit_to_content(hwnd: HWND) {
    unsafe {
        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        let height = content_height(scale) + (48.0 * scale) as i32; // the title bar and borders
        let _ = SetWindowPos(hwnd, None, 0, 0, (780.0 * scale) as i32, height, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let state = || unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
    match msg {
        WM_CREATE => {
            let create = lp.0 as *const CREATESTRUCTW;
            if !create.is_null() {
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize) };
            }
            fit_to_content(hwnd);
            LRESULT(0)
        }
        WM_PAINT => {
            let pointer = state();
            if pointer.is_null() {
                return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
            }
            let state = unsafe { &mut *pointer };
            let mut paint = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut paint) };
            paint_window(hwnd, dc, state);
            let _ = unsafe { EndPaint(hwnd, &paint) };
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let pointer = state();
            if pointer.is_null() {
                return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
            }
            let state = unsafe { &mut *pointer };
            let (x, y) = ((wp.0 & 0xFFFF) as i32, ((wp.0 >> 16) & 0xFFFF) as i32);
            let action = state.hits.iter().rev().find(|(rect, _)| inside(rect, x, y)).map(|(_, action)| *action);
            if let Some(action) = action {
                click(hwnd, state, action);
                let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            }
            LRESULT(0)
        }
        WM_TIMER => {
            let pointer = state();
            if !pointer.is_null() {
                let state = unsafe { &mut *pointer };
                // The file is the truth: a second copy of this window, or a hand edit, changes it.
                let fresh = launch::load();
                if fresh.run != state.settings.run || fresh.autostart != state.settings.autostart {
                    state.settings = fresh;
                }
                let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let pointer = state();
            if !pointer.is_null() {
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let state = Box::from_raw(pointer);
                    for font in state.fonts {
                        if !font.0.is_null() {
                            let _ = DeleteObject(font.into());
                        }
                    }
                }
            }
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn click(hwnd: HWND, state: &mut State, action: Action) {
    match action {
        Action::Toggle(id) => {
            let Some(component) = launch::COMPONENTS.iter().find(|component| component.id == id) else { return };
            let was = state.settings.wants(component);
            state.settings.set(id, !was);
            let _ = launch::save(&state.settings);
            launch::log(&format!("settings: {} {}", component.exe, if was { "off" } else { "on" }));
            if was {
                // Stopped by hand: the service would otherwise start it again.
                launch::close(component.exe);
            } else if !crate::supervise::active() {
                // Nobody is watching, and a switched-on component that never appears would be
                // a lie: start the service so the switch means what it says.
                if launch::start_service() {
                    launch::log("settings: started the service for a switched-on component");
                }
            }
        }
        Action::Autostart => {
            state.settings.autostart = !state.settings.autostart;
            let _ = launch::save(&state.settings);
            launch::logon::set(state.settings.autostart);
            launch::log(&format!("settings: autostart {}", if state.settings.autostart { "on" } else { "off" }));
        }
        Action::StartService => {
            if !launch::start_service() {
                launch::log("settings: the service did not start");
            }
        }
    }
    let _ = hwnd;
}

fn inside(rect: &RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

fn paint_window(hwnd: HWND, dc: HDC, state: &mut State) {
    let scale = state.rescale(hwnd);
    let px = |value: f32| (value * scale).round() as i32;
    let mut client = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return;
    }
    let width = client.right;

    let background = unsafe { CreateSolidBrush(BG) };
    unsafe { FillRect(dc, &client, background) };
    let _ = unsafe { DeleteObject(background.into()) };

    let mut y = px(20.0);
    let mut row = RECT { left: px(26.0), top: y, right: width - px(26.0), bottom: y + px(30.0) };
    draw(dc, TITLE, &mut row, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, TEXT, state.font(0));
    y += px(30.0);
    row = RECT { left: px(26.0), top: y, right: width - px(26.0), bottom: y + px(22.0) };
    draw(
        dc,
        "Служба запуска держит включённые компоненты запущенными и поднимает их при входе в Windows.",
        &mut row,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        TEXT_2,
        state.font(2),
    );
    y += px(32.0);

    state.hits.clear();
    for component in launch::COMPONENTS {
        paint_component(dc, state, component, &mut y, width, px);
    }

    // The logon switch: one entry for the whole suite.
    let check_row = RECT { left: px(26.0), top: y + px(4.0), right: width - px(26.0), bottom: y + px(34.0) };
    state.hits.push((check_row, Action::Autostart));
    let box = RECT { left: check_row.left, top: check_row.top + px(4.0), right: check_row.left + px(20.0), bottom: check_row.top + px(24.0) };
    let box_fill = unsafe { CreateSolidBrush(if state.settings.autostart { ACCENT } else { CARD_ON }) };
    unsafe { FillRect(dc, &box, box_fill) };
    let _ = unsafe { DeleteObject(box_fill.into()) };
    if state.settings.autostart {
        let mut tick = box;
        draw(dc, "✓", &mut tick, DT_CENTER | DT_SINGLELINE | DT_VCENTER, TEXT, state.font(2));
    }
    let mut label = RECT { left: box.right + px(12.0), top: check_row.top + px(2.0), right: check_row.right, bottom: check_row.top + px(26.0) };
    draw(dc, "Запускать при входе в Windows", &mut label, DT_LEFT | DT_SINGLELINE | DT_VCENTER, TEXT, state.font(2));
    y += px(48.0);

    // Is the service itself running?
    let watched = crate::supervise::active();
    let mut line = RECT { left: px(26.0), top: y, right: width - px(26.0), bottom: y + px(22.0) };
    let line_text = if watched {
        "Служба запущена: включённые компоненты под её присмотром."
    } else {
        "Служба не запущена: сейчас никто не следит за компонентами."
    };
    draw(dc, line_text, &mut line, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, if watched { RUNNING } else { TEXT_2 }, state.font(2));
    if !watched {
        let button = RECT { left: px(26.0), top: y + px(28.0), right: px(196.0), bottom: y + px(60.0) };
        let fill = unsafe { CreateSolidBrush(ACCENT) };
        unsafe { FillRect(dc, &button, fill) };
        let _ = unsafe { DeleteObject(fill.into()) };
        let mut caption = button;
        draw(dc, "Включить службу", &mut caption, DT_CENTER | DT_SINGLELINE | DT_VCENTER, TEXT, state.font(1));
        state.hits.push((button, Action::StartService));
    }

    // The settings file, so it can be found and edited.
    let footer_top = (y + px(96.0)).min(client.bottom - px(26.0));
    let mut footer = RECT { left: px(26.0), top: footer_top, right: width - px(26.0), bottom: footer_top + px(18.0) };
    draw(dc, &launch::settings_path().display().to_string(), &mut footer, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, TEXT_3, state.font(2));
}

fn paint_component(dc: HDC, state: &mut State, component: &Component, y: &mut i32, width: i32, px: impl Fn(f32) -> i32) {
    let card = RECT { left: px(26.0), top: *y, right: width - px(26.0), bottom: *y + px(74.0) };
    let wanted = state.settings.wants(component);
    let installed = launch::exe_of(component).is_some();
    let alive = installed && launch::running(component.exe);
    let fill = unsafe { CreateSolidBrush(if wanted { CARD_ON } else { CARD_OFF }) };
    unsafe { FillRect(dc, &card, fill) };
    let _ = unsafe { DeleteObject(fill.into()) };

    let text_right = card.right - px(126.0);
    let mut name = RECT { left: card.left + px(16.0), top: card.top + px(11.0), right: text_right, bottom: card.top + px(33.0) };
    draw(dc, component.name, &mut name, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, if wanted { TEXT } else { TEXT_2 }, state.font(1));
    let mut hint = RECT { left: card.left + px(16.0), top: card.top + px(33.0), right: text_right, bottom: card.top + px(53.0) };
    draw(dc, component.hint, &mut hint, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, TEXT_3, state.font(2));

    let (status, status_color) = if !installed {
        ("не установлен", TEXT_3)
    } else if alive {
        ("запущен", RUNNING)
    } else if wanted {
        ("жду запуска", ACCENT)
    } else {
        ("остановлен", TEXT_3)
    };
    let mut status_rect = RECT { left: card.left + px(16.0), top: card.bottom - px(21.0), right: text_right, bottom: card.bottom - px(5.0) };
    draw(dc, status, &mut status_rect, DT_LEFT | DT_SINGLELINE | DT_VCENTER, status_color, state.font(2));

    // The switch: this one button is the whole component's start setting.
    let button = RECT { left: card.right - px(110.0), top: card.top + px(21.0), right: card.right - px(16.0), bottom: card.top + px(53.0) };
    let button_fill = unsafe { CreateSolidBrush(if wanted { ACCENT_OFF } else { ACCENT }) };
    unsafe { FillRect(dc, &button, button_fill) };
    let _ = unsafe { DeleteObject(button_fill.into()) };
    let caption = if wanted { "Остановить" } else { "Запустить" };
    let mut caption_rect = button;
    draw(dc, caption, &mut caption_rect, DT_CENTER | DT_SINGLELINE | DT_VCENTER, if installed { TEXT } else { TEXT_3 }, state.font(1));
    if installed {
        state.hits.push((button, Action::Toggle(component.id)));
    }
    *y += px(84.0);
}

/// One line of text; the font and colour are put back afterwards.
fn draw(dc: HDC, text: &str, rect: &mut RECT, format: DRAW_TEXT_FORMAT, color: COLORREF, font: HGDIOBJ) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let previous = unsafe { SelectObject(dc, font) };
    unsafe { SetTextColor(dc, color) };
    unsafe { SetBkMode(dc, TRANSPARENT) };
    unsafe { DrawTextW(dc, &mut wide, rect, format) };
    unsafe { SelectObject(dc, previous) };
}
