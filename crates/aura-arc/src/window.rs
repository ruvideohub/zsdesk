//! The window `AuraArc` shows when it is started without a command.
//!
//! One archive at a time: a toolbar, the folder-like side list, the table of what is inside and
//! a status line — drawn with the same Direct2D canvas the rest of AuraUI uses, so the archiver
//! looks like one of the family rather than a console with a frame around it.
//!
//! Every action goes through the engine the command line uses, so the window cannot drift away
//! from the program.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use aura_windows::gfx::{Align, Color, Gfx, RectF, Surface, TextStyle};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, BitBlt, EndPaint, InvalidateRect, UpdateWindow, PAINTSTRUCT, SRCCOPY};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, KillTimer, LoadCursorW, LoadIconW, PostMessageW, PostQuitMessage, RegisterClassExW, SetTimer,
    SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage, CS_DBLCLKS, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HCURSOR, IDC_ARROW, MSG, SW_SHOW, WINDOW_EX_STYLE,
    WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DROPFILES, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_SIZE, WM_TIMER, WNDCLASSEXW,
    WS_OVERLAPPEDWINDOW,
};

use crate::zip;

/// Toolbar: what the window can do. The glyph is a Segoe MDL2 Assets code point.
const TOOLS: [(&str, &str); 8] = [
    ("\u{e8b7}", "Открыть"),
    // Создание архива: без этой кнопки пустое окно архиватора не давало ни одного пути к
    // новому файлу — «Добавить» и «Извлечь» работают только с уже открытым архивом.
    ("\u{e8f4}", "Новый архив"),
    ("\u{e710}", "Добавить"),
    ("\u{e896}", "Извлечь"),
    ("\u{e73e}", "Тест"),
    ("\u{e8c8}", "Копировать"),
    ("\u{e74d}", "Удалить"),
    ("\u{e713}", "Настройки"),
];

/// The side list: the same archive, seen through a filter.
const FILTERS: [(&str, &str); 5] = [
    ("\u{e8a5}", "Все файлы"),
    ("\u{e8b7}", "Папки"),
    ("\u{e8a5}", "Документы"),
    ("\u{e8b9}", "Медиа"),
    ("\u{e7b8}", "Архивы"),
];

/// Our own message: a worker thread has news for the window.
const WM_APP_WORK: u32 = 0x8000 + 1;
/// The window's one timer, which keeps the progress bar moving while a worker runs.
const TIMER: usize = 1;

const ROW_HEIGHT: f32 = 30.0;
const TOOLBAR_HEIGHT: f32 = 78.0;
const SEARCH_HEIGHT: f32 = 34.0;
const STATUS_HEIGHT: f32 = 34.0;
const SIDEBAR_WIDTH: f32 = 196.0;

/// Which part of the window the pointer is over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Tool(usize),
    Filter(usize),
    Row(usize),
    Search,
    SettingsToggle(usize),
    SettingsDone,
}

#[derive(Clone, Copy, PartialEq)]
struct Palette {
    background: Color,
    surface: Color,
    surface_hover: Color,
    divider: Color,
    text: Color,
    text_secondary: Color,
    accent: Color,
    field: Color,
}

const DARK: Palette = Palette {
    background: Color::rgba8(0x16, 0x1b, 0x21, 255),
    surface: Color::rgba8(0x1e, 0x24, 0x2b, 255),
    surface_hover: Color::rgba8(0x2a, 0x32, 0x3b, 255),
    divider: Color::rgba8(0x2c, 0x34, 0x3d, 255),
    text: Color::rgba8(0xe9, 0xed, 0xf2, 255),
    text_secondary: Color::rgba8(0x94, 0x9e, 0xaa, 255),
    accent: Color::rgba8(0x4f, 0xc3, 0xf7, 255),
    field: Color::rgba8(0x23, 0x2a, 0x32, 255),
};

const LIGHT: Palette = Palette {
    background: Color::rgba8(0xf7, 0xf8, 0xfa, 255),
    surface: Color::rgba8(0xff, 0xff, 0xff, 255),
    surface_hover: Color::rgba8(0xe9, 0xee, 0xf4, 255),
    divider: Color::rgba8(0xdc, 0xe1, 0xe7, 255),
    text: Color::rgba8(0x1b, 0x20, 0x26, 255),
    text_secondary: Color::rgba8(0x5f, 0x6b, 0x78, 255),
    accent: Color::rgba8(0x0a, 0x7f, 0xb8, 255),
    field: Color::rgba8(0xff, 0xff, 0xff, 255),
};

/// The two switches the archiver has. Saved next to the other AuraUI settings.
#[derive(Clone, Copy)]
struct Settings {
    compress: bool,
    open_after: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { compress: true, open_after: false }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        let base = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(base).join("AuraUI").join("AuraArc").join("settings.txt"))
    }
    fn load() -> Self {
        let mut settings = Self::default();
        let Some(text) = Self::path().and_then(|path| std::fs::read_to_string(path).ok()) else {
            return settings;
        };
        for line in text.lines() {
            match line.trim() {
                "compress=0" => settings.compress = false,
                "compress=1" => settings.compress = true,
                "open_after=1" => settings.open_after = true,
                "open_after=0" => settings.open_after = false,
                _ => {}
            }
        }
        settings
    }
    fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let text = format!(
            "compress={}\nopen_after={}\n",
            u8::from(self.compress),
            u8::from(self.open_after)
        );
        let _ = std::fs::write(path, text);
    }
}

/// What the window asks a worker thread to do. Everything the engine needs is moved in, so the
/// window stays answerable while a big archive is packed or unpacked.
enum Job {
    Add { archive: PathBuf, files: Vec<PathBuf>, store: bool },
    Extract { archive: PathBuf, destination: PathBuf, open_after: bool },
    Test { archive: PathBuf },
    Copy { archive: PathBuf, destination: PathBuf, names: Vec<String> },
    Remove { archive: PathBuf, names: Vec<String> },
}

/// What a worker tells the window: how far it has come, and what it ended up doing.
enum Done {
    Progress { done: usize, total: usize },
    Finished { status: String },
}

impl Job {
    fn begin(&self) -> String {
        match self {
            Job::Add { .. } => String::from("Упаковываю…"),
            Job::Extract { destination, .. } => format!("Распаковываю в {}…", destination.display()),
            Job::Test { .. } => String::from("Проверяю…"),
            Job::Copy { destination, .. } => format!("Копирую в {}…", destination.display()),
            Job::Remove { .. } => String::from("Убираю из архива…"),
        }
    }

    fn run(self, tell: &std::sync::mpsc::Sender<Done>) -> String {
        match self {
            Job::Add { archive, files, store } => {
                let mut done = 0usize;
                let outcome = zip::add(&archive, &files, store, |_, _| {
                    done += 1;
                    let _ = tell.send(Done::Progress { done, total: 0 });
                });
                match outcome {
                    Ok(summary) => {
                        let mut text = format!(
                            "Добавлено: {}, {}",
                            crate::items(summary),
                            crate::size(summary.packed)
                        );
                        if summary.skipped > 0 {
                            text.push_str(&format!(
                                ", уже было: {}",
                                crate::plural(summary.skipped, "запись", "записи", "записей")
                            ));
                        }
                        if summary.failed > 0 {
                            text.push_str(&format!(
                                ", не удалось: {}",
                                crate::plural(summary.failed, "файл", "файла", "файлов")
                            ));
                        }
                        text
                    }
                    Err(error) => format!("{error}"),
                }
            }
            Job::Extract { archive, destination, open_after } => {
                let total = zip::entries(&archive).map(|entries| entries.len()).unwrap_or(0);
                let mut done = 0usize;
                let outcome = zip::extract(&archive, &destination, |_, _| {
                    done += 1;
                    let _ = tell.send(Done::Progress { done, total });
                });
                match outcome {
                    Ok(summary) => {
                        if open_after {
                            open_folder(&destination);
                        }
                        let mut text = format!(
                            "Распаковано: {} в {}",
                            crate::items(summary),
                            destination.display()
                        );
                        if summary.failed > 0 {
                            text.push_str(&format!(
                                ", с ошибками: {}",
                                crate::plural(summary.failed, "файл", "файла", "файлов")
                            ));
                        }
                        text
                    }
                    Err(error) => format!("{error}"),
                }
            }
            Job::Test { archive } => {
                let total = zip::entries(&archive).map(|entries| entries.len()).unwrap_or(0);
                let mut done = 0usize;
                let outcome = zip::test(&archive, |_, _| {
                    done += 1;
                    let _ = tell.send(Done::Progress { done, total });
                });
                match outcome {
                    Ok(summary) if summary.failed == 0 => format!("Всё цело: {}", crate::items(summary)),
                    Ok(summary) => format!(
                        "Повреждено: {} из {}",
                        crate::plural(summary.failed, "файл", "файла", "файлов"),
                        summary.files + summary.failed
                    ),
                    Err(error) => format!("{error}"),
                }
            }
            Job::Copy { archive, destination, names } => {
                let total = names.len();
                let mut done = 0usize;
                let outcome = zip::extract_only(&archive, &destination, &names, |_, _| {
                    done += 1;
                    let _ = tell.send(Done::Progress { done, total });
                });
                match outcome {
                    Ok(summary) if summary.files + summary.folders > 0 => format!(
                        "Скопировано: {} в {}",
                        crate::items(summary),
                        destination.display()
                    ),
                    Ok(_) => String::from("Копировать нечего: выбранные записи пусты."),
                    Err(error) => format!("{error}"),
                }
            }
            Job::Remove { archive, names } => {
                let _ = tell.send(Done::Progress { done: 0, total: 0 });
                let name = archive
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match zip::remove(&archive, &names) {
                    Ok(summary) => format!("Убрано из «{name}»: {}", crate::items(summary)),
                    Err(error) => format!("{error}"),
                }
            }
        }
    }
}

struct App {
    hwnd: HWND,
    gfx: Option<Gfx>,
    surface: Option<Surface>,
    palette: Palette,
    archive: Option<PathBuf>,
    entries: Vec<zip::Entry>,
    visible: Vec<usize>,
    /// Rows picked in the table, by their position in `visible`.
    selected: BTreeSet<usize>,
    /// Where a Shift+click range starts.
    anchor: Option<usize>,
    filter: usize,
    query: String,
    scroll: f32,
    hover: Option<Hit>,
    settings: Settings,
    settings_open: bool,
    status: String,
    busy: bool,
    /// What the worker is saying, while there is one.
    work: Option<std::sync::mpsc::Receiver<Done>>,
    total: usize,
    done: usize,
    ticks: usize,
    width: f32,
    height: f32,
}

/// Where everything is. One pure function, used by both the painter and the mouse.
struct Layout {
    tools: Vec<RectF>,
    filters: Vec<RectF>,
    search: RectF,
    header: RectF,
    rows: RectF,
    columns: Vec<f32>,
    status: RectF,
    overlay: RectF,
    toggles: Vec<RectF>,
    done: RectF,
}

fn layout(width: f32, height: f32) -> Layout {
    let margin = 16.0;
    let mut tools = Vec::new();
    // The toolbar shares the width it has: seven tools at a fixed 92 px would overrun a narrow
    // window and sit under the search field.
    let tool_width = ((width - 2.0 * margin) / TOOLS.len() as f32).min(92.0);
    for index in 0..TOOLS.len() {
        tools.push(RectF::new(margin + index as f32 * tool_width, 10.0, tool_width - 8.0, TOOLBAR_HEIGHT - 22.0));
    }
    let search = RectF::new(width - margin - 260.0, TOOLBAR_HEIGHT + 10.0, 244.0, SEARCH_HEIGHT - 8.0);
    let body_top = TOOLBAR_HEIGHT + SEARCH_HEIGHT + 12.0;
    let mut filters = Vec::new();
    for index in 0..FILTERS.len() {
        filters.push(RectF::new(margin - 8.0, body_top + index as f32 * 36.0 - 2.0, SIDEBAR_WIDTH - 8.0, 32.0));
    }
    let list_left = margin + SIDEBAR_WIDTH;
    let header = RectF::new(list_left, body_top, (width - list_left - margin).max(10.0), 26.0);
    let rows = RectF::new(
        list_left,
        header.bottom() + 6.0,
        header.w,
        (height - header.bottom() - 6.0 - STATUS_HEIGHT - 6.0).max(10.0),
    );
    // Имя, Размер, Сжатый, Степень, Изменён — the last four share the right part of the row.
    let right = rows.right();
    let columns = vec![rows.x + 26.0, right - 300.0, right - 220.0, right - 140.0, right - 70.0];
    let status = RectF::new(0.0, height - STATUS_HEIGHT, width, STATUS_HEIGHT);
    let overlay = RectF::new((width - 380.0) / 2.0, (height - 210.0) / 2.0, 380.0, 210.0);
    let toggles = (0..2)
        .map(|index| RectF::new(overlay.x + 20.0, overlay.y + 56.0 + index as f32 * 44.0, overlay.w - 40.0, 34.0))
        .collect();
    let done = RectF::new(overlay.right() - 120.0, overlay.bottom() - 48.0, 100.0, 32.0);
    Layout { tools, filters, search, header, rows, columns, status, overlay, toggles, done }
}

fn style(size: f32) -> TextStyle {
    TextStyle::ui(size)
}

fn style_end(size: f32) -> TextStyle {
    TextStyle { align: Align::End, ..TextStyle::ui(size) }
}

fn style_center(size: f32) -> TextStyle {
    TextStyle { align: Align::Center, ..TextStyle::ui(size) }
}

impl App {
    fn reload(&mut self) {
        self.entries = match self.archive.as_ref() {
            Some(archive) => zip::entries(archive).unwrap_or_default(),
            None => Vec::new(),
        };
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        let query = self.query.to_lowercase();
        let filter = self.filter;
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| filter == 0 || kind_for(&entry.name) == filter)
            .filter(|(_, entry)| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
        self.selected.clear();
        self.anchor = None;
        self.scroll = 0.0;
    }

    fn totals(&self) -> (u64, u64) {
        let size: u64 = self.entries.iter().map(|entry| entry.size).sum();
        let packed: u64 = self.entries.iter().map(|entry| entry.packed).sum();
        (size, packed)
    }

    fn summary(&self) -> String {
        let (size, packed) = self.totals();
        if !self.selected.is_empty() {
            return format!(
                "Выбрано: {} ({})",
                crate::plural(self.selected.len(), "объект", "объекта", "объектов"),
                crate::size(self.selected_bytes())
            );
        }
        format!(
            "{}, {} в архиве, {} после сжатия",
            crate::plural(self.entries.len(), "объект", "объекта", "объектов"),
            crate::size(size),
            crate::size(packed),
        )
    }

    fn name(&self) -> String {
        self.archive
            .as_ref()
            .and_then(|archive| archive.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("архив не открыт"))
    }

    fn retitle(&self) {
        let text = format!("AuraArc — {}", self.name());
        let wide = crate::wide(&text);
        let _ = unsafe { SetWindowTextW(self.hwnd, PCWSTR(wide.as_ptr())) };
    }

    fn invalidate(&self) {
        unsafe { InvalidateRect(Some(self.hwnd), None, false) };
    }

    // ---- actions -------------------------------------------------------------

    fn open_dialog(&mut self) {
        let Some(files) = pick_files(self.hwnd, false, "Открыть архив") else { return };
        let Some(archive) = files.into_iter().next() else { return };
        self.load(archive);
    }

    /// «Новый архив»: где будет `.zip`, а потом сразу выбор файлов для него.
    ///
    /// Архив создаёт первое добавление, поэтому окно сначала открывает выбранный путь как
    /// новый архив («Новый архив: добавьте в него файлы») и лишь затем спрашивает файлы.
    /// Отмена выбора файлов оставляет пустое окно нового архива — ничего не ломается, и
    /// пользователь может нажать «Добавить» позже или перетащить файлы в окно.
    fn new_archive(&mut self) {
        let Some(path) = pick_new_archive(self.hwnd) else { return };
        self.load(path);
        self.add_files();
    }

    fn load(&mut self, archive: PathBuf) {
        self.archive = Some(archive.clone());
        if archive.exists() && !is_archive(&archive) {
            self.status = String::from("Это не архив .zip");
            self.entries.clear();
            self.apply_filter();
            self.retitle();
            self.invalidate();
            return;
        }
        self.reload();
        self.status = if archive.exists() {
            format!("{}: {}", self.name(), self.summary())
        } else {
            String::from("Новый архив: добавьте в него файлы.")
        };
        self.retitle();
        self.invalidate();
    }

    fn add_files(&mut self) {
        let Some(archive) = self.archive.clone() else { return };
        let Some(files) = pick_files(self.hwnd, true, "Добавить в архив") else { return };
        self.start(Job::Add { archive, files, store: !self.settings.compress });
    }

    fn add_paths(&mut self, archive: &Path, files: &[PathBuf]) {
        if files.is_empty() {
            return;
        }
        self.start(Job::Add {
            archive: archive.to_path_buf(),
            files: files.to_vec(),
            store: !self.settings.compress,
        });
    }

    fn extract(&mut self) {
        let Some(archive) = self.archive.clone() else { return };
        self.start(Job::Extract {
            destination: crate::beside(&archive),
            archive,
            open_after: self.settings.open_after,
        });
    }

    fn test(&mut self) {
        let Some(archive) = self.archive.clone() else { return };
        self.start(Job::Test { archive });
    }

    fn activate(&mut self, tool: usize) {
        match tool {
            0 => self.open_dialog(),
            1 => self.new_archive(),
            2 => self.add_files(),
            3 => self.extract(),
            4 => self.test(),
            5 => self.copy_selected(),
            6 => self.delete_selected(),
            _ => {
                self.settings_open = !self.settings_open;
                self.invalidate();
            }
        }
    }

    /// A tool is usable when it has something to work on; the row tools also need a row.
    fn enabled(&self, tool: usize) -> bool {
        match tool {
            // «Открыть» и «Новый архив» нужны именно тогда, когда архива ещё нет; настройки
            // не зависят ни от чего.
            0 | 1 | 7 => true,
            5 | 6 => !self.selected.is_empty(),
            _ => self.archive.is_some(),
        }
    }

    /// «Копировать»: the picked rows, unpacked beside the archive.
    fn copy_selected(&mut self) {
        let Some(archive) = self.archive.clone() else { return };
        let names = self.selected_names();
        if names.is_empty() {
            self.status = String::from("Сначала выберите запись в таблице.");
            self.invalidate();
            return;
        }
        let destination = crate::beside(&archive);
        self.start(Job::Copy { archive, destination, names });
    }

    /// «Удалить»: the picked rows go, everything else is copied over untouched.
    fn delete_selected(&mut self) {
        let Some(archive) = self.archive.clone() else { return };
        let names = self.selected_names();
        if names.is_empty() {
            self.status = String::from("Сначала выберите запись в таблице.");
            self.invalidate();
            return;
        }
        self.start(Job::Remove { archive, names });
    }

    /// The names behind the picked rows, in the order they appear.
    fn selected_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for position in &self.selected {
            let Some(index) = self.visible.get(*position) else { continue };
            let Some(entry) = self.entries.get(*index) else { continue };
            names.push(entry.name.clone());
        }
        names
    }

    fn selected_bytes(&self) -> u64 {
        let mut bytes = 0;
        for position in &self.selected {
            let Some(index) = self.visible.get(*position) else { continue };
            let Some(entry) = self.entries.get(*index) else { continue };
            bytes += entry.size;
        }
        bytes
    }

    // ---- the worker ----------------------------------------------------------

    /// Nothing in here blocks the window: the engine runs on its own thread and talks back
    /// through a channel, and the window is woken by a message of its own.
    fn start(&mut self, job: Job) {
        if self.busy {
            self.status = String::from("Подождите: идёт работа.");
            self.invalidate();
            return;
        }
        self.busy = true;
        self.total = 0;
        self.done = 0;
        self.ticks = 0;
        self.status = job.begin();
        let (tell, hear) = std::sync::mpsc::channel();
        self.work = Some(hear);
        let window = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let status = job.run(&tell);
            let _ = tell.send(Done::Finished { status });
            let hwnd = HWND(window as *mut core::ffi::c_void);
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_WORK, WPARAM(0), LPARAM(0));
            }
        });
        unsafe {
            SetTimer(Some(self.hwnd), TIMER, 60, None);
        }
        self.invalidate();
    }

    /// Reads whatever the worker has said since the last look and repaints.
    fn poll(&mut self) {
        self.ticks = self.ticks.wrapping_add(1);
        let Some(hear) = self.work.take() else { return };
        let mut finished = None;
        loop {
            match hear.try_recv() {
                Ok(Done::Progress { done, total }) => {
                    self.done = done;
                    if total > 0 {
                        self.total = total;
                    }
                }
                Ok(Done::Finished { status }) => {
                    finished = Some(status);
                    break;
                }
                Err(_) => break,
            }
        }
        match finished {
            Some(status) => {
                self.busy = false;
                self.status = status;
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER);
                }
                self.reload();
            }
            // Nothing yet: keep listening and keep the bar moving.
            None => self.work = Some(hear),
        }
        self.invalidate();
    }

    fn dropped(&mut self, drop: HDROP) {
        let mut paths = Vec::new();
        let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
        for index in 0..count {
            let length = unsafe { DragQueryFileW(drop, index, None) };
            if length == 0 {
                continue;
            }
            let mut buffer = vec![0u16; length as usize + 1];
            let copied = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) };
            if copied > 0 {
                paths.push(PathBuf::from(String::from_utf16_lossy(&buffer[..copied as usize])));
            }
        }
        unsafe { DragFinish(drop) };
        match paths.as_slice() {
            [] => {}
            [one] if is_archive(one) => self.load(one.clone()),
            _ => {
                if let Some(archive) = self.archive.clone() {
                    self.add_paths(&archive, &paths);
                } else if let Some(one) = paths.first() {
                    let folder = one.parent().map(Path::to_path_buf).unwrap_or_default();
                    let stem = one
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_else(|| String::from("архив"));
                    let archive = folder.join(format!("{stem}.zip"));
                    self.load(archive.clone());
                    self.add_paths(&archive, &paths);
                }
            }
        }
    }

    // ---- painting -----------------------------------------------------------

    fn draw(&self, cv: &aura_windows::gfx::Canvas) {
        let p = self.palette;
        let (width, height) = (self.width, self.height);
        let l = layout(width, height);
        cv.fill(RectF::new(0.0, 0.0, width, height), p.background);

        // Toolbar.
        for (index, rect) in l.tools.iter().enumerate() {
            let enabled = self.enabled(index);
            let hovered = self.hover == Some(Hit::Tool(index));
            if hovered && enabled {
                cv.round(*rect, 10.0, p.surface_hover);
            }
            let ink = if enabled { p.text } else { p.text_secondary };
            cv.glyph(TOOLS[index].0, RectF::new(rect.x, rect.y + 8.0, rect.w, 22.0), 20.0, ink);
            cv.text(TOOLS[index].1, RectF::new(rect.x, rect.y + 34.0, rect.w, 18.0), &style_center(12.5), ink);
        }
        cv.line(0.0, TOOLBAR_HEIGHT, width, TOOLBAR_HEIGHT, p.divider, 1.0);

        // Path and search.
        let path = self
            .archive
            .as_ref()
            .map(|archive| archive.display().to_string())
            .unwrap_or_else(|| String::from("архив не открыт"));
        cv.text(&path, RectF::new(16.0, TOOLBAR_HEIGHT + 10.0, width - 320.0, SEARCH_HEIGHT - 8.0), &style(13.0), p.text_secondary);
        cv.round(l.search, 8.0, p.field);
        cv.stroke_round(l.search, 8.0, if self.hover == Some(Hit::Search) { p.accent } else { p.divider }, 1.0);
        let search_text = if self.query.is_empty() { String::from("Поиск…") } else { self.query.clone() };
        let search_color = if self.query.is_empty() { p.text_secondary } else { p.text };
        cv.text(&search_text, RectF::new(l.search.x + 12.0, l.search.y, l.search.w - 40.0, l.search.h), &style(13.0), search_color);
        cv.glyph("\u{e721}", RectF::new(l.search.right() - 26.0, l.search.y, 18.0, l.search.h), 13.0, p.text_secondary);

        // Side list.
        for (index, rect) in l.filters.iter().enumerate() {
            let active = index == self.filter;
            if active {
                cv.round(*rect, 8.0, p.surface_hover);
            } else if self.hover == Some(Hit::Filter(index)) {
                cv.round(*rect, 8.0, p.surface);
            }
            let ink = if active { p.text } else { p.text_secondary };
            cv.glyph(FILTERS[index].0, RectF::new(rect.x + 12.0, rect.y, 18.0, rect.h), 14.0, ink);
            cv.text(FILTERS[index].1, RectF::new(rect.x + 38.0, rect.y, rect.w - 46.0, rect.h), &style(13.0), ink);
            if index > 0 {
                let count = self.entries.iter().filter(|entry| kind_for(&entry.name) == index).count();
                cv.text(&count.to_string(), RectF::new(rect.right() - 44.0, rect.y, 32.0, rect.h), &style_end(12.0), p.text_secondary);
            }
        }

        // Table header.
        let titles = ["Имя", "Размер", "Сжатый", "Степень", "Изменён"];
        cv.text(titles[0], RectF::new(l.columns[0], l.header.y, 200.0, l.header.h), &style(12.5), p.text_secondary);
        for index in 1..titles.len() {
            let width = (if index + 1 < titles.len() { l.columns[index + 1] } else { l.rows.right() }) - l.columns[index] - 8.0;
            cv.text(titles[index], RectF::new(l.columns[index], l.header.y, width, l.header.h), &style_end(12.5), p.text_secondary);
        }
        cv.line(l.rows.x - 8.0, l.header.bottom(), l.rows.right(), l.header.bottom(), p.divider, 1.0);

        // Table body.
        let capacity = (l.rows.h / ROW_HEIGHT).ceil() as usize;
        let first = (self.scroll / ROW_HEIGHT).floor() as usize;
        cv.push_clip(l.rows);
        for slot in 0..capacity.min(self.visible.len().saturating_sub(first)) {
            let position = first + slot;
            let Some(index) = self.visible.get(position).copied() else { break };
            let Some(entry) = self.entries.get(index) else { break };
            let y = l.rows.y + slot as f32 * ROW_HEIGHT - (self.scroll - first as f32 * ROW_HEIGHT);
            let row = RectF::new(l.rows.x - 6.0, y, l.rows.w + 6.0, ROW_HEIGHT);
            if self.selected.contains(&position) {
                cv.round(row.inset(2.0, 2.0), 8.0, p.surface_hover);
                cv.stroke_round(row.inset(2.0, 2.0), 8.0, p.accent, 1.0);
            } else if self.hover == Some(Hit::Row(position)) {
                cv.round(row.inset(2.0, 2.0), 8.0, p.surface);
            }
            let (glyph, tint) = icon_for(&entry.name);
            cv.round(RectF::new(l.columns[0] - 22.0, y + 7.0, 16.0, 16.0), 4.0, tint);
            cv.glyph(glyph, RectF::new(l.columns[0] - 22.0, y + 6.0, 16.0, 18.0), 10.0, Color::rgba8(0x10, 0x14, 0x18, 255));
            cv.text(&entry.name, RectF::new(l.columns[0], y, l.columns[1] - l.columns[0] - 16.0, ROW_HEIGHT), &style(13.0), p.text);
            if entry.is_folder() {
                cv.text("—", RectF::new(l.columns[1], y, l.columns[2] - l.columns[1] - 10.0, ROW_HEIGHT), &style_end(12.5), p.text_secondary);
                cv.text("—", RectF::new(l.columns[2], y, l.columns[3] - l.columns[2] - 10.0, ROW_HEIGHT), &style_end(12.5), p.text_secondary);
                cv.text("—", RectF::new(l.columns[3], y, l.columns[4] - l.columns[3] - 10.0, ROW_HEIGHT), &style_end(12.5), p.text_secondary);
            } else {
                cv.text(&crate::size(entry.size), RectF::new(l.columns[1], y, l.columns[2] - l.columns[1] - 10.0, ROW_HEIGHT), &style_end(12.5), p.text);
                cv.text(&crate::size(entry.packed), RectF::new(l.columns[2], y, l.columns[3] - l.columns[2] - 10.0, ROW_HEIGHT), &style_end(12.5), p.text_secondary);
                cv.text(&ratio(entry), RectF::new(l.columns[3], y, l.columns[4] - l.columns[3] - 10.0, ROW_HEIGHT), &style_end(12.5), p.text_secondary);
            }
            cv.text(&when(entry), RectF::new(l.columns[4], y, l.rows.right() - l.columns[4], ROW_HEIGHT), &style_end(12.5), p.text_secondary);
        }
        cv.pop_clip();

        if self.visible.is_empty() {
            let hint = if !self.entries.is_empty() {
                "Ничего не найдено"
            } else if self.archive.is_some() {
                "Архив пуст: «Добавить» положит в него файлы, или перетащите их на окно"
            } else {
                "Перетащите .zip на окно, нажмите «Открыть» — или «Новый архив», чтобы создать свой"
            };
            cv.text(hint, RectF::new(l.rows.x, l.rows.y + 40.0, l.rows.w, 24.0), &style_center(13.0), p.text_secondary);
        }

        // Status line.
        cv.line(0.0, l.status.y, width, l.status.y, p.divider, 1.0);
        let mut left = if self.status.is_empty() { self.summary() } else { self.status.clone() };
        if self.busy && self.total > 0 {
            left.push_str(&format!(" {} из {}", self.done.min(self.total), self.total));
        }
        cv.text(&left, RectF::new(16.0, l.status.y, width - 220.0, l.status.h), &style(12.5), p.text_secondary);
        let (_, packed) = self.totals();
        let (size, _) = self.totals();
        let method = if self.entries.iter().any(|entry| entry.method != zip::STORED) { "Deflate" } else { "Без сжатия" };
        let ratio_text = if size > 0 { format!("{} • {}%", method, packed * 100 / size) } else { String::from(method) };
        cv.text(&ratio_text, RectF::new(width - 200.0, l.status.y, 184.0, l.status.h), &style_end(12.5), p.accent);
        // While a worker runs the window says so with a bar, not with a frozen picture.
        if self.busy {
            let bar = RectF::new(l.status.x + 16.0, l.status.bottom() - 6.0, width - 32.0, 3.0);
            cv.round(bar, 1.5, p.field);
            if self.total > 0 {
                let done = (self.done.min(self.total)) as f32 / self.total as f32;
                if done > 0.0 {
                    cv.round(RectF::new(bar.x, bar.y, bar.w * done, bar.h), 1.5, p.accent);
                }
            } else {
                // Unknown length: a segment sweeping from left to right, moved by the timer.
                let span = bar.w * 0.3;
                let travel = bar.w - span;
                let phase = (self.ticks % 40) as f32 / 40.0;
                cv.round(RectF::new(bar.x + travel * phase, bar.y, span, bar.h), 1.5, p.accent);
            }
        }

        // Settings overlay.
        if self.settings_open {
            cv.fill(RectF::new(0.0, 0.0, width, height), Color::rgba8(0, 0, 0, 130));
            cv.round(l.overlay, 14.0, p.surface);
            cv.stroke_round(l.overlay, 14.0, p.divider, 1.0);
            cv.text("Настройки архиватора", RectF::new(l.overlay.x + 20.0, l.overlay.y + 16.0, l.overlay.w - 40.0, 24.0), &style(15.0), p.text);
            let labels = [
                ("Сжимать файлы", "выключено — архив пишется без сжатия, быстрее"),
                ("Открывать папку", "после распаковки показать результат в проводнике"),
            ];
            for (index, rect) in l.toggles.iter().enumerate() {
                let on = if index == 0 { self.settings.compress } else { self.settings.open_after };
                if self.hover == Some(Hit::SettingsToggle(index)) {
                    cv.round(*rect, 8.0, p.surface_hover);
                }
                cv.text(labels[index].0, RectF::new(rect.x + 8.0, rect.y, rect.w - 40.0, 20.0), &style(13.0), p.text);
                cv.text(labels[index].1, RectF::new(rect.x + 8.0, rect.y + 16.0, rect.w - 40.0, 16.0), &style(11.5), p.text_secondary);
                let toggle = RectF::new(rect.right() - 26.0, rect.y + 6.0, 18.0, 18.0);
                cv.round(toggle, 5.0, if on { p.accent } else { p.field });
                cv.stroke_round(toggle, 5.0, if on { p.accent } else { p.divider }, 1.0);
                if on {
                    cv.glyph("\u{e73e}", toggle, 12.0, Color::rgba8(0x10, 0x14, 0x18, 255));
                }
            }
            let hovered = self.hover == Some(Hit::SettingsDone);
            cv.round(l.done, 8.0, if hovered { p.surface_hover } else { p.field });
            cv.stroke_round(l.done, 8.0, p.divider, 1.0);
            cv.text("Готово", l.done, &style_center(13.0), p.text);
        }
    }

    fn paint(&mut self) {
        let mut client = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut client) }.is_err() {
            return;
        }
        self.width = (client.right - client.left) as f32;
        self.height = (client.bottom - client.top) as f32;
        let width = self.width as i32;
        let height = self.height as i32;
        if width <= 0 || height <= 0 {
            return;
        }
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(self.hwnd, &mut paint) };
        if Surface::ensure(&mut self.surface, width, height).is_some() {
            if let (Some(gfx), Some(surface)) = (self.gfx.as_ref(), self.surface.as_ref()) {
                let drawn = gfx.draw(surface, |cv| self.draw(cv));
                if drawn.is_ok() {
                    unsafe {
                        let _ = BitBlt(dc, 0, 0, width, height, Some(surface.dc), 0, 0, SRCCOPY);
                    }
                }
            }
        }
        let _ = unsafe { EndPaint(self.hwnd, &paint) };
    }

    // ---- pointer ------------------------------------------------------------

    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let l = layout(self.width, self.height);
        if self.settings_open {
            for (index, rect) in l.toggles.iter().enumerate() {
                if rect.contains(x, y) {
                    return Some(Hit::SettingsToggle(index));
                }
            }
            if l.done.contains(x, y) {
                return Some(Hit::SettingsDone);
            }
            return None;
        }
        for (index, rect) in l.tools.iter().enumerate() {
            if rect.contains(x, y) {
                return Some(Hit::Tool(index));
            }
        }
        if l.search.contains(x, y) {
            return Some(Hit::Search);
        }
        for (index, rect) in l.filters.iter().enumerate() {
            if rect.contains(x, y) {
                return Some(Hit::Filter(index));
            }
        }
        if l.rows.contains(x, y) {
            let offset = y - l.rows.y + (self.scroll - (self.scroll / ROW_HEIGHT).floor() * ROW_HEIGHT);
            let first = (self.scroll / ROW_HEIGHT).floor() as usize;
            let position = first + (offset / ROW_HEIGHT) as usize;
            if position < self.visible.len() {
                return Some(Hit::Row(position));
            }
        }
        None
    }

    fn click(&mut self, x: f32, y: f32, control: bool, shift: bool) {
        match self.hit(x, y) {
            Some(Hit::Tool(index)) => self.activate(index),
            Some(Hit::Filter(index)) => {
                self.filter = index;
                self.status.clear();
                self.reload();
                self.invalidate();
            }
            Some(Hit::Row(position)) => {
                if control {
                    if !self.selected.remove(&position) {
                        self.selected.insert(position);
                    }
                    self.anchor = Some(position);
                } else if shift {
                    let anchor = self.anchor.unwrap_or(position);
                    let (from, to) = if anchor <= position { (anchor, position) } else { (position, anchor) };
                    for index in from..=to {
                        self.selected.insert(index);
                    }
                } else {
                    self.selected.clear();
                    self.selected.insert(position);
                    self.anchor = Some(position);
                }
                self.status.clear();
                self.invalidate();
            }
            Some(Hit::SettingsToggle(index)) => {
                if index == 0 {
                    self.settings.compress = !self.settings.compress;
                } else {
                    self.settings.open_after = !self.settings.open_after;
                }
                self.settings.save();
                self.invalidate();
            }
            Some(Hit::SettingsDone) => {
                self.settings_open = false;
                self.invalidate();
            }
            Some(Hit::Search) | None => {}
        }
    }

    fn wheel(&mut self, delta: i32) {
        let l = layout(self.width, self.height);
        let page = (l.rows.h / ROW_HEIGHT).floor() * ROW_HEIGHT;
        let max = (self.visible.len() as f32 * ROW_HEIGHT - page).max(0.0);
        self.scroll = (self.scroll - delta as f32 / 120.0 * ROW_HEIGHT * 3.0).clamp(0.0, max);
        self.invalidate();
    }

    fn key(&mut self, code: usize, control: bool) {
        if self.settings_open {
            if code == 0x1b {
                self.settings_open = false;
                self.invalidate();
            }
            return;
        }
        match code {
            0x1b => {
                if self.query.is_empty() {
                    let _ = unsafe { DestroyWindow(self.hwnd) };
                } else {
                    self.query.clear();
                    self.status.clear();
                    self.reload();
                    self.invalidate();
                }
            }
            0x08 => {
                self.query.pop();
                self.status.clear();
                self.reload();
                self.invalidate();
            }
            // Ctrl+O, Ctrl+E, F5 — the shortcuts every archiver shares.
            0x4f if control => self.activate(0),
            0x45 if control => self.activate(2),
            0x74 => self.activate(3),
            // Delete, with a row picked: the same thing the «Удалить» tool does.
            0x2e => {
                if !self.selected.is_empty() {
                    self.delete_selected();
                }
            }
            0x41 if control => {
                self.selected = (0..self.visible.len()).collect();
                self.anchor = self.visible.first().copied();
                self.status.clear();
                self.invalidate();
            }
            _ => {}
        }
    }

    fn typed(&mut self, character: char) {
        if self.settings_open || character.is_control() {
            return;
        }
        if self.query.len() < 120 {
            self.status.clear();
            self.query.push(character);
            self.reload();
            self.invalidate();
        }
    }
}

fn kind_for(name: &str) -> usize {
    let lowered = name.to_lowercase();
    if lowered.ends_with('/') {
        return 1;
    }
    let extension = lowered.rsplit('.').next().unwrap_or("");
    match extension {
        "zip" | "7z" | "rar" | "gz" | "xz" | "bz2" | "tar" => 4,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "mp4" | "mkv" | "mp3" | "wav" | "flac" => 3,
        "txt" | "md" | "pdf" | "doc" | "docx" | "rtf" | "odt" | "xls" | "xlsx" | "csv" => 2,
        _ => 0,
    }
}

fn icon_for(name: &str) -> (&'static str, Color) {
    match kind_for(name) {
        1 => ("\u{e8b7}", Color::rgba8(0xf5, 0xc2, 0x4c, 255)),
        2 => ("\u{e8a5}", Color::rgba8(0x5c, 0x9d, 0xf5, 255)),
        3 => ("\u{e8b9}", Color::rgba8(0xb2, 0x8c, 0xf0, 255)),
        4 => ("\u{e7b8}", Color::rgba8(0xf0, 0x8c, 0x5c, 255)),
        _ => ("\u{e7c3}", Color::rgba8(0x7f, 0xbf, 0x8f, 255)),
    }
}

/// `35%` — how much smaller the entry became.
fn ratio(entry: &zip::Entry) -> String {
    if entry.size == 0 {
        return String::from("—");
    }
    format!("{}%", entry.packed.saturating_mul(100) / entry.size)
}

/// The stored moment as a person reads it: `20.06.2026 14:05`.
fn when(entry: &zip::Entry) -> String {
    let day = entry.date & 0x1f;
    let month = (entry.date >> 5) & 0x0f;
    let year = 1980 + (entry.date >> 9);
    if day == 0 || month == 0 {
        return String::from("—");
    }
    format!(
        "{day:02}.{month:02}.{year} {:02}:{:02}",
        entry.time >> 11,
        (entry.time >> 5) & 0x3f
    )
}

fn is_archive(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn open_folder(path: &Path) {
    let target = crate::wide(&path.display().to_string());
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(target.as_ptr()),
            None,
            None,
            SW_SHOW,
        );
    }
}

/// The system picker: several files for «Добавить», one for «Открыть».
fn pick_files(owner: HWND, several: bool, title: &str) -> Option<Vec<PathBuf>> {
    let mut buffer = vec![0u16; 4096];
    let filter: Vec<u16> = crate::wide("Архивы (*.zip)\0*.zip\0Все файлы\0*.*\0\0");
    let caption = crate::wide(title);
    let mut flags = OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if several {
        flags |= OFN_ALLOWMULTISELECT | OFN_FILEMUSTEXIST;
    }
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: PCWSTR(caption.as_ptr()),
        Flags: flags,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut dialog) }.as_bool() {
        return None;
    }
    Some(claim_paths(&buffer))
}

/// The system picker for a *new* archive: a name to save under, with `.zip` filled in.
fn pick_new_archive(owner: HWND) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 4096];
    let suggestion = crate::wide("Архив.zip");
    buffer[..suggestion.len()].copy_from_slice(&suggestion);
    let filter: Vec<u16> = crate::wide("Архивы (*.zip)\0*.zip\0Все файлы\0*.*\0\0");
    let caption = crate::wide("Новый архив");
    let extension = crate::wide("zip");
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: PCWSTR(caption.as_ptr()),
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_OVERWRITEPROMPT,
        ..Default::default()
    };
    if !unsafe { GetSaveFileNameW(&mut dialog) }.as_bool() {
        return None;
    }
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    let mut path = PathBuf::from(String::from_utf16_lossy(&buffer[..end]));
    // Клавиатурный ввод без расширения — обычное дело; архиватор без `.zip` не откроет.
    if !path.to_ascii_lowercase().to_string_lossy().ends_with(".zip") {
        path.set_extension("zip");
    }
    Some(path)
}

/// The picker hands back one name, or a folder followed by the names chosen inside it.
fn claim_paths(buffer: &[u16]) -> Vec<PathBuf> {
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    let first = PathBuf::from(String::from_utf16_lossy(&buffer[..end]));
    let mut rest: Vec<PathBuf> = Vec::new();
    let mut at = end + 1;
    while at < buffer.len() && buffer[at] != 0 {
        let stop = buffer[at..]
            .iter()
            .position(|unit| *unit == 0)
            .map(|offset| at + offset)
            .unwrap_or(buffer.len());
        rest.push(PathBuf::from(String::from_utf16_lossy(&buffer[at..stop])));
        at = stop + 1;
    }
    if rest.is_empty() {
        vec![first]
    } else {
        rest.into_iter().map(|name| first.join(name)).collect()
    }
}

/// Dark title bar and rounded corners: the frame belongs to Windows, but it can match the app.
fn dress(hwnd: HWND, dark: bool) {
    aura_windows::theme::set_dark_frame(hwnd, dark);
    aura_windows::theme::round_corners(hwnd, false);
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
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
            // MK_CONTROL and MK_SHIFT come with the click; asking for the keyboard state later
            // would race with the person's own fingers.
            let control = wp.0 & 0x0008 != 0;
            let shift = wp.0 & 0x0004 != 0;
            app.click(x, y, control, shift);
            LRESULT(0)
        }
        WM_APP_WORK => {
            app.poll();
            LRESULT(0)
        }
        WM_TIMER => {
            if app.busy {
                app.poll();
            }
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            let x = (lp.0 & 0xffff) as i16 as f32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
            if let Some(Hit::Row(_)) = app.hit(x, y) {
                if !app.busy {
                    app.extract();
                }
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xffff) as i16 as i32;
            app.wheel(delta);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let control = (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
            app.key(wp.0 & 0xffff, control);
            LRESULT(0)
        }
        WM_CHAR => {
            if let Some(character) = char::from_u32((wp.0 & 0xffff) as u32) {
                app.typed(character);
            }
            LRESULT(0)
        }
        WM_DROPFILES => {
            if !app.busy {
                app.dropped(HDROP(lp.0 as *mut _));
            }
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

/// Opens the archive window; returns the exit code the shell reads.
pub fn run(archive: Option<PathBuf>) -> i32 {
    unsafe {
        // Without arguments the program is being used as a window, so the console it was
        // started with only gets in the way.
        let console = windows::Win32::System::Console::GetConsoleWindow();
        if !console.0.is_null() {
            let _ = ShowWindow(console, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
        }
        // Direct2D factories want a COM-initialised thread: this is the UI thread.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let instance = GetModuleHandleW(None)
            .map(|module| HINSTANCE(module.0))
            .unwrap_or(HINSTANCE(std::ptr::null_mut()));
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR(std::ptr::null_mut())),
            lpszClassName: w!("AuraArcWindow"),
            // Значок из ресурса (resources/aura-arc.rc, id 101): иначе у окна и на панели
            // задач остаётся стандартный значок Windows.
            hIcon: LoadIconW(Some(instance), PCWSTR(101usize as *const u16)).unwrap_or_default(),
            hIconSm: LoadIconW(Some(instance), PCWSTR(101usize as *const u16)).unwrap_or_default(),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return 1;
        }
        let dark = aura_windows::theme::system_dark_mode();
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("AuraArcWindow"),
            w!("AuraArc — архиватор AuraUI"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            860,
            560,
            None,
            None,
            Some(instance),
            None,
        )
        .unwrap_or(HWND(std::ptr::null_mut()));
        if window.0.is_null() {
            return 1;
        }
        let state = Box::new(App {
            hwnd: window,
            gfx: Gfx::new("Segoe UI").ok(),
            surface: None,
            palette: if dark { DARK } else { LIGHT },
            archive: None,
            entries: Vec::new(),
            visible: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            filter: 0,
            query: String::new(),
            scroll: 0.0,
            hover: None,
            settings: Settings::load(),
            settings_open: false,
            status: String::new(),
            busy: false,
            work: None,
            total: 0,
            done: 0,
            ticks: 0,
            width: 860.0,
            height: 560.0,
        });
        let pointer = Box::into_raw(state);
        SetWindowLongPtrW(window, GWLP_USERDATA, pointer as isize);
        DragAcceptFiles(window, true);
        dress(window, dark);
        let app = &mut *pointer;
        app.retitle();
        let _ = ShowWindow(window, SW_SHOW);
        let _ = UpdateWindow(window);
        if let Some(archive) = archive {
            app.load(archive);
        }
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0);
            if result.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if com {
            CoUninitialize();
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_chosen_file_is_one_path() {
        let mut buffer: Vec<u16> = "C:\\работа\\отпуск.zip".encode_utf16().collect();
        buffer.push(0);
        assert_eq!(claim_paths(&buffer), vec![PathBuf::from("C:\\работа\\отпуск.zip")]);
    }

    #[test]
    fn several_chosen_files_are_found_under_their_folder() {
        let mut buffer: Vec<u16> = "C:\\работа".encode_utf16().collect();
        buffer.push(0);
        for name in ["а.txt", "б.txt"] {
            buffer.extend(name.encode_utf16());
            buffer.push(0);
        }
        buffer.push(0);
        assert_eq!(
            claim_paths(&buffer),
            vec![PathBuf::from("C:\\работа\\а.txt"), PathBuf::from("C:\\работа\\б.txt")]
        );
    }

    #[test]
    fn a_zip_is_recognised_whatever_its_case() {
        assert!(is_archive(Path::new("C:\\отпуск.ZIP")));
        assert!(!is_archive(Path::new("C:\\отпуск.7z")));
    }

    #[test]
    fn the_side_list_sorts_names_by_kind() {
        assert_eq!(kind_for("папка/"), 1);
        assert_eq!(kind_for("фото.png"), 3);
        assert_eq!(kind_for("заметка.md"), 2);
        assert_eq!(kind_for("архив.zip"), 4);
        assert_eq!(kind_for("программа.exe"), 0);
    }
}
