//! The settings panel of AuraUI — the one window that says what runs and where the rest is set.
//!
//! It is drawn, not built out of controls: the same Direct2D canvas and the same numbers as the
//! dock's own settings (`aura_windows::settings`), so the suite has one look. The panel itself
//! knows only what the launcher knows — which components exist, whether they are running, and
//! which program answers `--settings`; every component's own settings are its own business and
//! open as its own window.
//!
//! Three sections, the way the dock's settings are built: «Компоненты» (a switch each),
//! «Программы» (open, and the way to the program's settings), «Запуск» (logon, the service, the
//! log). Every action reports what happened in the line at the bottom and in `auraservice.log`.

use std::time::{Duration, Instant};

use aura_windows::gfx::{Align, Gfx, RectF, Surface, TextStyle};
use aura_windows::settings::{self as page, Look, Side};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, BitBlt, EndPaint, InvalidateRect, PAINTSTRUCT, SRCCOPY};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, KillTimer, LoadCursorW, PostQuitMessage, RegisterClassW, SetTimer,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, CS_DBLCLKS,
    CS_HREDRAW, CS_VREDRAW, SWP_NOACTIVATE, SWP_NOZORDER,
    CW_USEDEFAULT, GWLP_USERDATA, IDC_ARROW, MSG, SW_SHOW, WINDOW_EX_STYLE, WM_CLOSE,
    WM_DESTROY, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_SIZE, WM_TIMER,
    WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use crate::launch::{self, Settings};

/// Text as the wide strings Windows takes, ending in a zero.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

const CLASS: PCWSTR = w!("AuraServiceSettings");
const TITLE: &str = "AuraUI — настройки и запуск";
const FACE: &str = "Segoe UI";
/// The clock that keeps the picture true: a component may be started or closed by anyone.
const TIMER: usize = 1;
/// How long a component may take to appear before the window says it did not.
const SPAWN_WAIT: Duration = Duration::from_secs(8);
/// The page is opened at this size, in logical pixels, and grows with the display's scale.
const PAGE: (f32, f32) = (720.0, 520.0);
/// `WM_DPICHANGED`: the window moved to a display with another scale.
const WM_DPICHANGED: u32 = 0x02E0;

/// The sections, in the order the dock's settings put theirs.
///
/// «Программы» listed the archiver. With it gone the list is empty, and a section that can
/// only ever be empty is not a section: it comes back with the first program that belongs
/// there, and nothing else in this file has to change.
const TABS: [&str; 2] = ["Компоненты", "Запуск"];
const TAB_COMPONENTS: usize = 0;
const TAB_START: usize = 1;
/// Where the program list would sit, kept for the day it has an entry again.
const _TAB_PROGRAMS: usize = 1;

/// What the pointer is over, and therefore what a click does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Tab(usize),
    Done,
    Toggle(usize),
    ComponentSettings(usize),
    OpenProgram(usize),
    ProgramSettings(usize),
    Logon,
    Service,
    Log,
}

/// The colour of a sentence at the bottom.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Info,
    Ok,
    Bad,
}

struct App {
    hwnd: HWND,
    /// Whether the system is in its dark theme — the palette follows it, and a DPI change
    /// rebuilds the palette, so it is remembered rather than asked for again.
    dark: bool,
    gfx: Option<Gfx>,
    surface: Option<Surface>,
    look: Look,
    settings: Settings,
    tab: usize,
    hover: Option<Act>,
    /// The last thing that happened, and how it went.
    notice: String,
    tone: Tone,
    /// Components just switched on, and when the window stops waiting for them.
    waiting: Vec<(&'static str, Instant)>,
    service_wait: Option<Instant>,
    width: f32,
    height: f32,
}

/// Where everything is this frame, and what each control does. One pure function, used by the
/// painter and by the mouse — a control drawn at a place is exactly the control clicked at it.
struct Layout {
    done: RectF,
    tray: RectF,
    tabs: Vec<RectF>,
    card: RectF,
    rows: Vec<RowLayout>,
    footer: RectF,
}

struct RowLayout {
    rect: RectF,
    /// The text column, already narrowed by whatever stands on the right.
    text: RectF,
    /// A switch at the right edge.
    toggle: Option<(RectF, Act)>,
    /// Buttons at the right edge, each with what it does.
    buttons: Vec<(RectF, Act)>,
}

/// Does this component have a settings page it can open itself?
fn has_settings(index: usize) -> bool {
    launch::COMPONENTS.get(index).map(|component| !component.settings.is_empty()).unwrap_or(false)
}

fn row_count(tab: usize) -> usize {
    match tab {
        TAB_COMPONENTS => launch::COMPONENTS.len(),
        TAB_START => 3,
        _ => launch::PROGRAMS.len(),
    }
}

fn layout(look: &Look, width: f32, height: f32, tab: usize) -> Layout {
    let page = RectF::new(0.0, 0.0, width, height);
    let done = page::done_rect(look, page, "Готово");
    let tray = page::tray_rect(look, look.px(page::PAD), look.px(70.0), TABS.len());
    let tabs = page::tab_rects(look, look.px(page::PAD), look.px(70.0), TABS.len());
    let top = tray.bottom() + look.px(22.0);
    let count = row_count(tab);
    let card = look.section_card(page, top, count);
    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let rect = page::row_rect(look, card, index);
        // «Запуск» mixes a switch (logon) with buttons (service, log): each row says its own.
        let (toggle, buttons, side) = match (tab, index) {
            // A component with a settings page of its own gets the way to it beside its switch;
            // one without gets nothing, rather than a button that opens nothing.
            (TAB_COMPONENTS, _) if has_settings(index) => (
                Some((page::toggle_rect(look, rect), Act::Toggle(index))),
                vec![(page::toggle_side_button(look, rect), Act::ComponentSettings(index))],
                Side::ToggleAndButton,
            ),
            (TAB_COMPONENTS, _) => (
                Some((page::toggle_rect(look, rect), Act::Toggle(index))),
                Vec::new(),
                Side::Toggle,
            ),
            // No tab reaches this branch while PROGRAMS is empty; it stays so a program
            // added later gets its two buttons without touching the layout.
            (_, _) if !launch::PROGRAMS.is_empty() && tab == _TAB_PROGRAMS => (
                None,
                vec![
                    (page::button_rect(look, rect, 1, 2), Act::OpenProgram(index)),
                    (page::button_rect(look, rect, 0, 2), Act::ProgramSettings(index)),
                ],
                Side::TwoButtons,
            ),
            (TAB_START, 0) => (Some((page::toggle_rect(look, rect), Act::Logon)), Vec::new(), Side::Toggle),
            (TAB_START, 1) => (None, vec![(page::button_rect(look, rect, 0, 1), Act::Service)], Side::Button),
            _ => (None, vec![(page::button_rect(look, rect, 0, 1), Act::Log)], Side::Button),
        };
        rows.push(RowLayout { rect, text: page::row_text(look, rect, side), toggle, buttons });
    }
    let footer = RectF::new(
        look.px(page::PAD),
        height - look.px(34.0),
        (width - look.px(page::PAD) * 2.0).max(10.0),
        look.px(20.0),
    );
    Layout { done, tray, tabs, card, rows, footer }
}

impl App {
    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Which action a point belongs to.
    fn hit(&self, x: f32, y: f32) -> Option<Act> {
        let l = layout(&self.look, self.width, self.height, self.tab);
        for (index, rect) in l.tabs.iter().enumerate() {
            if rect.contains(x, y) {
                return Some(Act::Tab(index));
            }
        }
        if l.done.contains(x, y) {
            return Some(Act::Done);
        }
        for row in &l.rows {
            if let Some((field, act)) = row.toggle {
                // A switch is small: give it a comfortable margin to be clicked in.
                if field.inset(-self.look.px(8.0), -self.look.px(8.0)).contains(x, y) {
                    return Some(act);
                }
            }
            for (field, act) in &row.buttons {
                if field.contains(x, y) {
                    return Some(*act);
                }
            }
        }
        None
    }

    fn click(&mut self, x: f32, y: f32) {
        match self.hit(x, y) {
            Some(Act::Tab(index)) => {
                self.tab = index;
                self.invalidate();
            }
            Some(Act::Done) => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            Some(Act::Toggle(index)) => self.toggle_component(index),
            Some(Act::ComponentSettings(index)) => self.open_component_settings(index),
            Some(Act::OpenProgram(index)) => self.open_program(index, false),
            Some(Act::ProgramSettings(index)) => self.open_program(index, true),
            Some(Act::Logon) => self.toggle_logon(),
            Some(Act::Service) => self.toggle_service(),
            Some(Act::Log) => self.open_log(),
            None => {}
        }
    }

    // ---- actions ---------------------------------------------------------------

    fn say(&mut self, text: impl Into<String>, tone: Tone) {
        self.notice = text.into();
        self.tone = tone;
        self.invalidate();
    }

    /// One switch per component: «пусть работает». Turning it off also closes the program,
    /// because a switch that says off while the program runs is a lie.
    fn toggle_component(&mut self, index: usize) {
        let Some(component) = launch::COMPONENTS.get(index) else { return };
        let was = self.settings.wants(component);
        self.settings.set(component.id, !was);
        if launch::save(&self.settings).is_err() {
            self.say("Не удалось сохранить настройки: проверьте папку настроек.", Tone::Bad);
            return;
        }
        launch::log(&format!("settings: {} {}", component.exe, if was { "off" } else { "on" }));
        if was {
            let closed = launch::close(component.exe);
            let line = if closed {
                format!("{}: выключен и закрыт.", component.name)
            } else {
                format!("{}: выключен (закрывать было нечего).", component.name)
            };
            self.say(line, Tone::Info);
            return;
        }
        // Nothing is watching yet: the switch would be a lie without the service.
        if !crate::supervise::active() {
            launch::start_service();
            self.service_wait = Some(Instant::now());
        }
        self.waiting.push((component.id, Instant::now() + SPAWN_WAIT));
        self.say(format!("{}: включаю…", component.name), Tone::Info);
    }

    /// «Настройки…» on a component's row: the component opens its own page, in its own window.
    fn open_component_settings(&mut self, index: usize) {
        let Some(component) = launch::COMPONENTS.get(index) else { return };
        if component.settings.is_empty() {
            self.say(format!("{}: собственных настроек пока нет.", component.name), Tone::Info);
            return;
        }
        if launch::start_component_with(component, component.settings) {
            self.say(
                format!("{}: открываю настройки (в своём окне — там есть и «Готово»).", component.name),
                Tone::Info,
            );
        } else {
            self.say(
                format!("{}: не найден рядом со службой — переустановите AuraUI.", component.name),
                Tone::Bad,
            );
        }
    }

    fn toggle_logon(&mut self) {
        let wanted = !self.settings.autostart;
        self.settings.autostart = wanted;
        let saved = launch::save(&self.settings).is_ok();
        let written = launch::logon::set(wanted);
        launch::log(&format!("settings: autostart {}", if wanted { "on" } else { "off" }));
        let (text, tone) = if !saved {
            ("Не удалось сохранить настройки: проверьте папку настроек.", Tone::Bad)
        } else if !written {
            ("Настройки сохранены, но запись автозапуска не удалась (ключ Run в реестре).", Tone::Bad)
        } else if wanted {
            ("Автозапуск включён: при входе в Windows запускается служба.", Tone::Ok)
        } else {
            ("Автозапуск выключен.", Tone::Info)
        };
        self.say(text, tone);
    }

    fn toggle_service(&mut self) {
        if crate::supervise::active() {
            launch::close("auraservice.exe");
            launch::log("settings: service stopped by hand");
            self.say("Служба остановлена. Компоненты останутся как есть.", Tone::Info);
        } else if launch::start_service() {
            self.service_wait = Some(Instant::now());
            launch::log("settings: service start requested");
            self.say("Запускаю службу…", Tone::Info);
        } else {
            launch::log("settings: the service could not be started");
            self.say("Не удалось запустить auraservice.exe --run: смотрите журнал.", Tone::Bad);
        }
    }

    fn open_program(&mut self, index: usize, wants_settings: bool) {
        let Some(program) = launch::PROGRAMS.get(index) else { return };
        let arguments: &[&str] = if wants_settings { program.settings } else { &[] };
        if wants_settings && arguments.is_empty() {
            self.say(format!("{}: собственных настроек пока нет.", program.name), Tone::Info);
            return;
        }
        if launch::start_program(program, arguments) {
            let text = if wants_settings {
                format!("{}: открываю настройки.", program.name)
            } else {
                format!("{}: открываю.", program.name)
            };
            self.say(text, Tone::Info);
        } else {
            self.say(
                format!("{}: не найден рядом со службой — переустановите AuraUI.", program.name),
                Tone::Bad,
            );
        }
    }

    fn open_log(&mut self) {
        let target = if launch::log_path().is_file() {
            launch::log_path()
        } else {
            launch::dir()
        };
        let wide = wide(&target.display().to_string());
        let opened = unsafe {
            windows::Win32::UI::Shell::ShellExecuteW(
                Some(self.hwnd),
                w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            )
        };
        if opened.0 as isize <= 32 {
            self.say(format!("Путь журнала: {}", launch::log_path().display()), Tone::Info);
        }
    }

    // ---- the clock -------------------------------------------------------------

    /// Once a second: read the world again and follow up on what was asked for.
    fn tick(&mut self) {
        let fresh = launch::load();
        if fresh.run != self.settings.run || fresh.autostart != self.settings.autostart {
            self.settings = fresh;
        }
        let mut answered: Vec<(&'static str, bool)> = Vec::new();
        self.waiting.retain(|(id, deadline)| {
            let Some(component) = launch::COMPONENTS.iter().find(|component| component.id == *id) else {
                return false;
            };
            if launch::running(component.exe) {
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
                self.say(format!("{name}: запущен."), Tone::Ok);
            } else {
                self.say(
                    format!("{name}: не запустился. Причина — в журнале {}", launch::log_path().display()),
                    Tone::Bad,
                );
            }
        }
        if let Some(since) = self.service_wait {
            if crate::supervise::active() {
                self.service_wait = None;
                self.say("Служба запущена.", Tone::Ok);
            } else if since.elapsed() > SPAWN_WAIT {
                self.service_wait = None;
                self.say(
                    format!("Служба не запустилась. Причина — в журнале {}", launch::log_path().display()),
                    Tone::Bad,
                );
            }
        }
        self.invalidate();
    }

    // ---- painting --------------------------------------------------------------

    fn draw(&self, cv: &aura_windows::gfx::Canvas) {
        let look = &self.look;
        let page = RectF::new(0.0, 0.0, self.width, self.height);
        page::background(cv, look, page);
        page::header(cv, look, page, "Настройки AuraUI", "Готово", self.hover == Some(Act::Done));
        let l = layout(look, self.width, self.height, self.tab);
        page::tabs(cv, look, l.tray, &l.tabs, &TABS, self.tab);
        page::card(cv, look, l.card);

        match self.tab {
            TAB_COMPONENTS => self.draw_components(cv, look, &l),
            // The program page exists only while there is a program to list; with the list
            // empty its tab is not drawn either.
            _ => self.draw_start(cv, look, &l),
        }

        let tone = match self.tone {
            Tone::Ok => look.ok,
            Tone::Bad => look.bad,
            Tone::Info => look.text2,
        };
        let notice = if self.notice.is_empty() { self.resting_line() } else { self.notice.clone() };
        page::footer(cv, look, page, &notice, tone);
        // The settings file is named in the corner: «nothing is saved» should have a place to look.
        let path = launch::settings_path().display().to_string();
        let style = TextStyle::ui(look.font(10.5)).align(Align::End);
        cv.text(&path, RectF::new(l.footer.x, l.footer.y - look.px(16.0), l.footer.w, look.px(16.0)), &style, look.text3);
    }

    fn resting_line(&self) -> String {
        let active = crate::supervise::active();
        if active {
            String::from("Служба запущена: включённые компоненты под её присмотром.")
        } else {
            String::from("Служба не запущена: сейчас никто не следит за компонентами.")
        }
    }

    fn draw_components(&self, cv: &aura_windows::gfx::Canvas, look: &Look, l: &Layout) {
        for (index, component) in launch::COMPONENTS.iter().enumerate() {
            let Some(row) = l.rows.get(index) else { break };
            if let Some(line) = page::row_separator(look, l.card, index) {
                page::separator(cv, line, look.hair);
            }
            let installed = launch::exe_of(component).is_some();
            let alive = installed && launch::running(component.exe);
            let wanted = self.settings.wants(component);
            let (status, tone) = if !installed {
                (format!("не установлен: {} нет в папке программы", component.exe), look.bad)
            } else if alive {
                (String::from("запущен"), look.ok)
            } else if wanted {
                (String::from("включён, жду запуска…"), look.warn)
            } else {
                (String::from("остановлен"), look.text3)
            };
            page::row_text_painted(cv, look, row.text, component.name, &format!("{} · {status}", component.hint), tone);
            if let Some((field, act)) = row.toggle {
                page::toggle(cv, look, field, wanted, self.hover == Some(act), installed);
            }
            for (field, act) in &row.buttons {
                page::button(cv, look, *field, "Настройки…", self.hover == Some(*act), installed);
            }
        }
    }

    /// Draws the program list. Reachable again as soon as `launch::PROGRAMS` has an entry;
    /// until then the tab is not drawn, so nothing calls this and the compiler would warn.
    #[allow(dead_code)]
    fn draw_programs(&self, cv: &aura_windows::gfx::Canvas, look: &Look, l: &Layout) {
        for (index, program) in launch::PROGRAMS.iter().enumerate() {
            let Some(row) = l.rows.get(index) else { break };
            if let Some(line) = page::row_separator(look, l.card, index) {
                page::separator(cv, line, look.hair);
            }
            let installed = launch::program_exe(program).is_some();
            let has_settings = !program.settings.is_empty();
            page::row_text_painted(cv, look, row.text, program.name, program.hint, look.text2);
            for (field, act) in &row.buttons {
                let (label, enabled) = match act {
                    Act::OpenProgram(_) => ("Открыть", installed),
                    _ => ("Настройки…", installed && has_settings),
                };
                page::button(cv, look, *field, label, self.hover == Some(*act), enabled);
            }
        }
    }

    fn draw_start(&self, cv: &aura_windows::gfx::Canvas, look: &Look, l: &Layout) {
        let active = crate::supervise::active();
        for index in 0..3 {
            let Some(row) = l.rows.get(index) else { break };
            if let Some(line) = page::row_separator(look, l.card, index) {
                page::separator(cv, line, look.hair);
            }
            match index {
                0 => {
                    page::row_text_painted(
                        cv,
                        look,
                        row.text,
                        "Запускать при входе в Windows",
                        "одна запись автозапуска — служба, она и поднимает компоненты",
                        look.text2,
                    );
                    if let Some((field, act)) = row.toggle {
                        page::toggle(cv, look, field, self.settings.autostart, self.hover == Some(act), true);
                    }
                }
                1 => {
                    let hint = if active {
                        "служба запущена: держит включённые компоненты и поднимает их при падении"
                    } else {
                        "служба не запущена: никто не следит за компонентами"
                    };
                    page::row_text_painted(cv, look, row.text, "Служба запуска", hint, if active { look.ok } else { look.warn });
                    for (field, act) in &row.buttons {
                        let label = if active { "Остановить службу" } else { "Включить службу" };
                        page::accent_button(cv, look, *field, label, self.hover == Some(*act), true);
                    }
                }
                _ => {
                    let hint = launch::log_path().display().to_string();
                    page::row_text_painted(cv, look, row.text, "Журнал службы", &hint, look.text2);
                    for (field, act) in &row.buttons {
                        page::button(cv, look, *field, "Открыть", self.hover == Some(*act), true);
                    }
                }
            }
        }
    }

    fn paint(&mut self) {
        let mut client = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut client) }.is_err() {
            return;
        }
        self.width = (client.right - client.left) as f32;
        self.height = (client.bottom - client.top) as f32;
        let (width, height) = (self.width as i32, self.height as i32);
        if width <= 0 || height <= 0 {
            return;
        }
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(self.hwnd, &mut paint) };
        if Surface::ensure(&mut self.surface, width, height).is_some() {
            if let (Some(gfx), Some(surface)) = (self.gfx.as_ref(), self.surface.as_ref()) {
                if gfx.draw(surface, |cv| self.draw(cv)).is_ok() {
                    unsafe {
                        let _ = BitBlt(dc, 0, 0, width, height, Some(surface.dc), 0, 0, SRCCOPY);
                    }
                }
            }
        }
        let _ = unsafe { EndPaint(self.hwnd, &paint) };
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_NCDESTROY {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if !state.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(state));
        }
        return DefWindowProcW(hwnd, message, wp, lp);
    }
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if state.is_null() {
        return DefWindowProcW(hwnd, message, wp, lp);
    }
    let app = &mut *state;
    match message {
        WM_SIZE => {
            app.invalidate();
            LRESULT(0)
        }
        WM_PAINT => {
            app.paint();
            LRESULT(0)
        }
        WM_TIMER => {
            app.tick();
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // `wParam` carries the new DPI in its low word; `lParam` the rectangle Windows
            // suggests, already inside the new display's work area.
            let dpi = (wp.0 & 0xffff) as f32;
            if dpi >= 96.0 {
                let suggested = lp.0 as *const RECT;
                if !suggested.is_null() {
                    let rect = unsafe { *suggested };
                    unsafe {
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            rect.left,
                            rect.top,
                            rect.right - rect.left,
                            rect.bottom - rect.top,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
                app.look = Look::of(app.dark, dpi / 96.0);
                app.invalidate();
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lp.0 & 0xffff) as i16 as f32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
            let hit = app.hit(x, y);
            if hit != app.hover {
                app.hover = hit;
                app.invalidate();
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lp.0 & 0xffff) as i16 as f32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
            app.click(x, y);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wp, lp),
    }
}

/// Open the settings window; returns when it is closed.
pub fn run() -> windows::core::Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Direct2D factories want a COM-initialised thread: this is the UI thread.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let module = GetModuleHandleW(None)?;
        let instance = HINSTANCE(module.0);
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
        let dark = aura_windows::theme::system_dark_mode();
        let scale = GetDpiForSystem() as f32 / 96.0;
        let width = (PAGE.0 * scale).round() as i32;
        let height = (PAGE.1 * scale).round() as i32;
        let caption = wide(TITLE);
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            PCWSTR(caption.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            width,
            height,
            None,
            None,
            Some(instance),
            None,
        )?;
        let scale = GetDpiForWindow(window) as f32 / 96.0;
        let state = Box::new(App {
            hwnd: window,
            dark,
            gfx: Gfx::new(FACE).ok(),
            surface: None,
            look: Look::of(dark, scale),
            settings: launch::load(),
            tab: TAB_COMPONENTS,
            hover: None,
            notice: String::new(),
            tone: Tone::Info,
            waiting: Vec::new(),
            service_wait: None,
            width: width as f32,
            height: height as f32,
        });
        let pointer = Box::into_raw(state);
        SetWindowLongPtrW(window, GWLP_USERDATA, pointer as isize);
        aura_windows::theme::set_dark_frame(window, dark);
        aura_windows::theme::round_corners(window, false);
        let title = wide(TITLE);
        let _ = SetWindowTextW(window, PCWSTR(title.as_ptr()));
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetTimer(Some(window), TIMER, 1000, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = KillTimer(Some(window), TIMER);
        if com {
            CoUninitialize();
        }
    }
    Ok(())
}
