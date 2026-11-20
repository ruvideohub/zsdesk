//! The launch settings window.
//!
//! One row per component: what it is, whether it runs right now, and the one switch that says
//! "keep it running". Plus the logon switch and the state of the service that does the work.
//!
//! The controls are real Windows controls (`BUTTON`, `STATIC`), not shapes painted by hand:
//! a click is reported by the system as `WM_COMMAND`, so it cannot be lost to a hit-test, and
//! the button itself shows that it was pressed. Every action reports its outcome in the status
//! line at the bottom and in `auraservice.log`, so "nothing happens" always has a reason.

use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR, Result};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateFontIndirectW, DeleteObject, HFONT, LOGFONTW};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_PUSHBUTTON};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnableWindow, GetClientRect, GetMessageW, GetWindowLongPtrW,
    PostQuitMessage, RegisterClassW, SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage,
    CREATESTRUCTW, GWLP_USERDATA, MSG, SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_SIZE, WM_SETFONT, WM_TIMER, WNDCLASSW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_CLIPCHILDREN, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};

use crate::launch::{self, Component, Settings};

const CLASS: PCWSTR = w!("AuraServiceSettings");
const TITLE: &str = "AuraUI — запуск и компоненты";
const TIMER: usize = 1;
/// How long a component may take to appear before the window says it did not.
const SPAWN_WAIT: Duration = Duration::from_secs(8);

const FACE: &str = "Segoe UI";

/// One component's row of controls.
struct Row {
    index: usize,
    name: HWND,
    hint: HWND,
    status: HWND,
    button: HWND,
}

struct State {
    settings: Settings,
    rows: Vec<Row>,
    /// "Запускать при входе в Windows".
    logon: HWND,
    /// The line about the service itself.
    service: HWND,
    /// "Включить службу" / "Остановить службу".
    service_button: HWND,
    /// The result of the last action, and why it failed when it did.
    notice: HWND,
    /// "Открыть журнал".
    log_button: HWND,
    footer: HWND,
    title: HWND,
    subtitle: HWND,
    fonts: [HFONT; 2],
    /// A component that was just switched on, and the moment the window stops waiting for it.
    waiting: Vec<(&'static str, Instant)>,
    /// A service start that was requested and has not been seen yet.
    service_wait: Option<Instant>,
    scale: f32,
}

impl State {
    fn font(&self, index: usize) -> HFONT {
        self.fonts[index]
    }
}

fn make_font(scale: f32, size: f32, weight: i32) -> HFONT {
    let mut log = LOGFONTW { lfHeight: -(size * scale).round() as i32, lfWeight: weight, ..Default::default() };
    let family: Vec<u16> = FACE.encode_utf16().collect();
    let length = family.len().min(log.lfFaceName.len());
    log.lfFaceName[..length].copy_from_slice(&family[..length]);
    unsafe { CreateFontIndirectW(&log) }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn set_text(control: HWND, text: &str) {
    if control.0.is_null() {
        return;
    }
    unsafe {
        let _ = SetWindowTextW(control, PCWSTR(wide(text).as_ptr()));
    }
}

fn enable(control: HWND, on: bool) {
    if !control.0.is_null() {
        let _ = unsafe { EnableWindow(control, on) };
    }
}

fn set_checked(control: HWND, checked: bool) {
    if !control.0.is_null() {
        unsafe {
            SendMessageW(control, BM_SETCHECK, Some(WPARAM(usize::from(checked))), None);
        }
    }
}

fn create_control(parent: HWND, class: PCWSTR, text: &str, id: usize, style: WINDOW_STYLE) -> HWND {
    let text = wide(text);
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(text.as_ptr()),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            10,
            10,
            Some(parent),
            Some(windows::Win32::Foundation::HMENU(id as *mut std::ffi::c_void)),
            None,
            None,
        )
        .unwrap_or_default()
    }
}

/// Open the settings window; returns when it is closed.
pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let module = GetModuleHandleW(None)?;
        let background = windows::Win32::Graphics::Gdi::HBRUSH((windows::Win32::Graphics::Gdi::COLOR_BTNFACE.0 + 1) as usize as *mut _);
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: HINSTANCE(module.0),
            lpszClassName: CLASS,
            hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_ARROW)?,
            hbrBackground: background,
            ..Default::default()
        };
        RegisterClassW(&class);
        let state = Box::new(State {
            settings: launch::load(),
            rows: Vec::new(),
            logon: HWND::default(),
            service: HWND::default(),
            service_button: HWND::default(),
            notice: HWND::default(),
            log_button: HWND::default(),
            footer: HWND::default(),
            title: HWND::default(),
            subtitle: HWND::default(),
            fonts: [HFONT::default(); 2],
            waiting: Vec::new(),
            service_wait: None,
            scale: 1.0,
        });
        let pointer = Box::into_raw(state);
        let caption = wide(TITLE);
        let hwnd = match CreateWindowExW(
            WS_EX_APPWINDOW,
            CLASS,
            PCWSTR(caption.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            160,
            140,
            700,
            620,
            None,
            None,
            Some(HINSTANCE(module.0)),
            Some(pointer.cast()),
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                drop(Box::from_raw(pointer));
                return Err(error);
            }
        };
        fit(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetTimer(Some(hwnd), TIMER, 1000, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Ok(())
    }
}

/// Size the window to its content once, so no row is cut off and no empty space is left.
fn fit(hwnd: HWND) {
    let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
    let rows = launch::COMPONENTS.len() as f32;
    let client = 16.0 + 30.0 + 30.0 + rows * 68.0 + 6.0 + 34.0 + 40.0 + 20.0 + 24.0;
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            (700.0 * scale) as i32,
            (client * scale) as i32 + (56.0 * scale) as i32,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
        )
    };
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let state = || unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
    match msg {
        WM_CREATE => {
            let create = lp.0 as *const CREATESTRUCTW;
            if !create.is_null() {
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize) };
                let state = unsafe { &mut *((*create).lpCreateParams as *mut State) };
                build(hwnd, state);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let pointer = state();
            if !pointer.is_null() {
                layout(hwnd, unsafe { &mut *pointer });
            }
            LRESULT(0)
        }
        WM_TIMER => {
            let pointer = state();
            if !pointer.is_null() {
                let state = unsafe { &mut *pointer };
                tick(state);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let pointer = state();
            if pointer.is_null() {
                return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
            }
            let state = unsafe { &mut *pointer };
            let control = HWND(lp.0 as *mut std::ffi::c_void);
            // The switch is identified by the number the control was created with.
            let id = (wp.0 & 0xFFFF) as usize;
            let event = (wp.0 >> 16) as u32;
            // BN_CLICKED is 0; a checkbox also reports BN_CLICKED when toggled by the user.
            if event == 0 {
                command(state, id, control);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
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

/// Control ids: 100 is the logon switch, 101 the service button, 200+ the component rows.
const ID_LOGON: usize = 100;
const ID_SERVICE: usize = 101;
const ID_LOG: usize = 102;
const ID_ROW: usize = 200;

/// Create the controls once, then let `layout` place them.
fn build(hwnd: HWND, state: &mut State) {
    let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
    state.scale = scale;
    state.fonts = [make_font(scale, 16.5, 600), make_font(scale, 9.0, 400)];
    let body = state.font(1);
    let title = state.font(0);

    state.title = create_control(hwnd, w!("STATIC"), TITLE, 1, WINDOW_STYLE::default());
    state.subtitle = create_control(
        hwnd,
        w!("STATIC"),
        "Служба запуска держит включённые компоненты запущенными и поднимает их при входе в Windows.",
        2,
        WINDOW_STYLE::default(),
    );
    for (index, component) in launch::COMPONENTS.iter().enumerate() {
        let id = ID_ROW + index;
        state.rows.push(Row {
            index,
            name: create_control(hwnd, w!("STATIC"), component.name, id * 4, WINDOW_STYLE::default()),
            hint: create_control(hwnd, w!("STATIC"), component.hint, id * 4 + 1, WINDOW_STYLE::default()),
            status: create_control(hwnd, w!("STATIC"), "", id * 4 + 2, WINDOW_STYLE::default()),
            button: create_control(
                hwnd,
                w!("BUTTON"),
                "Запустить",
                id,
                WINDOW_STYLE(BS_PUSHBUTTON as u32) | WS_TABSTOP,
            ),
        });
    }
    state.logon = create_control(
        hwnd,
        w!("BUTTON"),
        "Запускать при входе в Windows",
        ID_LOGON,
        WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP,
    );
    state.service = create_control(hwnd, w!("STATIC"), "", 3, WINDOW_STYLE::default());
    state.service_button = create_control(
        hwnd,
        w!("BUTTON"),
        "Включить службу",
        ID_SERVICE,
        WINDOW_STYLE(BS_PUSHBUTTON as u32) | WS_TABSTOP,
    );
    state.notice = create_control(hwnd, w!("STATIC"), "Готово.", 4, WINDOW_STYLE::default());
    state.log_button = create_control(
        hwnd,
        w!("BUTTON"),
        "Открыть журнал",
        ID_LOG,
        WINDOW_STYLE(BS_PUSHBUTTON as u32) | WS_TABSTOP,
    );
    state.footer = create_control(hwnd, w!("STATIC"), &launch::settings_path().display().to_string(), 5, WINDOW_STYLE::default());

    for control in [
        state.title,
        state.subtitle,
        state.logon,
        state.service,
        state.service_button,
        state.notice,
        state.footer,
        state.log_button,
    ] {
        let font = if control == state.title { title } else { body };
        unsafe { SendMessageW(control, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1))) };
    }
    for row in &state.rows {
        for control in [row.name, row.hint, row.status] {
            unsafe { SendMessageW(control, WM_SETFONT, Some(WPARAM(body.0 as usize)), Some(LPARAM(1))) };
        }
        unsafe { SendMessageW(row.button, WM_SETFONT, Some(WPARAM(body.0 as usize)), Some(LPARAM(1))) };
    }
    layout(hwnd, state);
    refresh(state);
}

/// Place everything for the client size the window has now.
fn layout(hwnd: HWND, state: &mut State) {
    let mut client = windows::Win32::Foundation::RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return;
    }
    let k = state.scale.max(1.0);
    let px = |value: f32| (value * k).round() as i32;
    let width = client.right;
    let margin = px(20.0);
    let button_width = px(120.0);
    let button_height = px(30.0);
    let text_width = (width - margin * 2 - button_width - px(16.0)).max(px(120.0));

    let mut y = px(16.0);
    move_to(state.title, margin, y, width - margin * 2, px(26.0));
    y += px(30.0);
    move_to(state.subtitle, margin, y, width - margin * 2, px(20.0));
    y += px(30.0);

    for row in &state.rows {
        move_to(row.name, margin, y + px(4.0), text_width, px(20.0));
        move_to(row.hint, margin, y + px(24.0), text_width, px(18.0));
        move_to(row.status, margin, y + px(42.0), text_width, px(18.0));
        move_to(row.button, width - margin - button_width, y + px(14.0), button_width, button_height);
        y += px(68.0);
    }

    y += px(6.0);
    move_to(state.logon, margin, y, width - margin * 2, px(24.0));
    y += px(34.0);
    move_to(state.service, margin, y + px(6.0), text_width, px(20.0));
    move_to(state.service_button, width - margin - button_width, y, button_width, button_height);
    y += px(40.0);
    move_to(state.notice, margin, y, text_width, px(20.0));
    move_to(state.log_button, width - margin - button_width, y - px(6.0), button_width, button_height);
    move_to(state.footer, margin, client.bottom - px(24.0), width - margin * 2, px(18.0));
}

fn move_to(control: HWND, x: i32, y: i32, width: i32, height: i32) {
    if control.0.is_null() {
        return;
    }
    let _ = unsafe {
        SetWindowPos(
            control,
            None,
            x,
            y,
            width,
            height,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
        )
    };
}

/// Read the world and write it into the controls. Called after every action and once a second.
fn refresh(state: &mut State) {
    for row in &state.rows {
        let component = &launch::COMPONENTS[row.index];
        let installed = launch::exe_of(component).is_some();
        let alive = installed && launch::running(component.exe);
        let wanted = state.settings.wants(component);
        set_text(row.button, if wanted { "Остановить" } else { "Запустить" });
        enable(row.button, installed);
        let status = if !installed {
            format!("не установлен: {} нет в папке программы", component.exe)
        } else if alive {
            "запущен".to_string()
        } else if wanted {
            "включён, жду запуска…".to_string()
        } else {
            "остановлен".to_string()
        };
        set_text(row.status, &status);
    }
    set_checked(state.logon, state.settings.autostart);
    let active = crate::supervise::active();
    set_text(
        state.service,
        if active {
            "Служба запущена: включённые компоненты под её присмотром."
        } else {
            "Служба не запущена: сейчас никто не следит за компонентами."
        },
    );
    set_text(state.service_button, if active { "Остановить службу" } else { "Включить службу" });
}

/// Once a second: refresh the picture and follow up on what was asked for.
fn tick(state: &mut State) {
    let fresh = launch::load();
    if fresh.run != state.settings.run || fresh.autostart != state.settings.autostart {
        state.settings = fresh;
    }
    let mut answered: Vec<(&'static str, bool)> = Vec::new();
    state.waiting.retain(|(id, deadline)| {
        let Some(component) = launch::COMPONENTS.iter().find(|component| component.id == *id) else { return false };
        let alive = launch::running(component.exe);
        if alive {
            answered.push((component.name, true));
            return false;
        }
        if Instant::now() >= *deadline {
            answered.push((component.name, false));
            return false;
        }
        true
    });
    for (name, ok) in answered {
        if ok {
            set_text(state.notice, &format!("{name}: запущен."));
        } else {
            set_text(
                state.notice,
                &format!("{name}: не запустился. Причина — в журнале {}", launch::log_path().display()),
            );
        }
    }
    if let Some(since) = state.service_wait {
        let active = crate::supervise::active();
        if active {
            state.service_wait = None;
            set_text(state.notice, "Служба запущена.");
        } else if since.elapsed() > SPAWN_WAIT {
            state.service_wait = None;
            set_text(
                state.notice,
                &format!("Служба не запустилась. Причина — в журнале {}", launch::log_path().display()),
            );
        }
    }
    refresh(state);
}

fn command(state: &mut State, id: usize, control: HWND) {
    if id == ID_LOGON {
        // A checkbox toggles itself before telling us, so read the new state from the control.
        // A checkbox flips itself before telling us, so read the state it now has.
        let checked = (unsafe { SendMessageW(state.logon, BM_GETCHECK, None, None) }.0 & 0xFFFF) == 1;
        state.settings.autostart = checked;
        let saved = launch::save(&state.settings).is_ok();
        let written = launch::logon::set(checked);
        launch::log(&format!("settings: autostart {}", if checked { "on" } else { "off" }));
        set_text(
            state.notice,
            if !saved {
                "Не удалось сохранить настройки: проверьте папку настроек."
            } else if !written {
                "Настройки сохранены, но запись автозапуска не удалась (ключ Run в реестре)."
            } else if checked {
                "Автозапуск включён: при входе в Windows запускается служба."
            } else {
                "Автозапуск выключен."
            },
        );
        refresh(state);
        return;
    }
    if id == ID_SERVICE {
        if crate::supervise::active() {
            launch::close("auraservice.exe");
            launch::log("settings: service stopped by hand");
            set_text(state.notice, "Служба остановлена. Компоненты останутся как есть.");
        } else if launch::start_service() {
            state.service_wait = Some(Instant::now());
            launch::log("settings: service start requested");
            set_text(state.notice, "Запускаю службу…");
        } else {
            launch::log("settings: the service could not be started");
            set_text(state.notice, "Не удалось запустить auraservice.exe --run: смотрите журнал.");
        }
        refresh(state);
        return;
    }
    if id == ID_LOG {
        let path: Vec<u16> = launch::log_path().display().to_string().encode_utf16().chain(Some(0)).collect();
        // The log may not exist yet; the folder is still worth opening in that case.
        let target: Vec<u16> = if launch::log_path().is_file() {
            path
        } else {
            launch::dir().display().to_string().encode_utf16().chain(Some(0)).collect()
        };
        let opened = unsafe {
            windows::Win32::UI::Shell::ShellExecuteW(
                Some(state.notice),
                w!("open"),
                PCWSTR(target.as_ptr()),
                None,
                None,
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            )
        };
        if opened.0 as isize <= 32 {
            set_text(state.notice, &format!("Путь журнала: {}", launch::log_path().display()));
        }
        return;
    }
    if id < ID_ROW {
        let _ = control;
        return;
    }
    let index = id - ID_ROW;
    let Some(component) = launch::COMPONENTS.get(index) else { return };
    let was = state.settings.wants(component);
    state.settings.set(component.id, !was);
    if launch::save(&state.settings).is_err() {
        set_text(state.notice, "Не удалось сохранить настройки: проверьте папку настроек.");
        refresh(state);
        return;
    }
    launch::log(&format!("settings: {} {}", component.exe, if was { "off" } else { "on" }));
    if was {
        let closed = launch::close(component.exe);
        set_text(
            state.notice,
            if closed {
                format!("{}: выключен и закрыт.", component.name)
            } else {
                format!("{}: выключен (закрывать было нечего).", component.name)
            },
        );
    } else {
        // Nothing is watching yet: the switch would be a lie without the service.
        if !crate::supervise::active() {
            launch::start_service();
            state.service_wait = Some(Instant::now());
        }
        state.waiting.push((component.id, Instant::now() + SPAWN_WAIT));
        set_text(state.notice, &format!("{}: включаю…", component.name));
    }
    refresh(state);
}

/// Kept for the compiler's sake: `Component` is used through the rows above.
#[allow(dead_code)]
fn _component(_: &Component) {}
