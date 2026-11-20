//! The window `AuraArc` shows when it is started without a command.
//!
//! One archive at a time, four buttons and a list — the archiver a person who does not use a
//! console can still use. Everything it does goes through the same engine the command line
//! uses, so the window cannot drift away from the program.

use std::path::{Path, PathBuf};

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{BOOL, COLORREF, HCURSOR, HINSTANCE, HMENU, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, GetStockObject, SetBkColor, SetTextColor,
    DEFAULT_GUI_FONT, HBRUSH, HDC, HGDIOBJ, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, LoadCursorW, MoveWindow, PostQuitMessage, RegisterClassExW, SendMessageW,
    SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage, UpdateWindow, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, IDC_ARROW, LBS_NOINTEGRALHEIGHT, MSG, SW_SHOW,
    WINDOW_EX_STYLE, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC,
    WM_DESTROY, WM_DROPFILES, WM_KEYDOWN, WM_NCDESTROY, WM_PAINT, WM_SIZE, WNDCLASSEXW, WS_CHILD, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::zip;

const OPEN: usize = 1001;
const ADD: usize = 1002;
const EXTRACT: usize = 1003;
const TEST: usize = 1004;

/// Colours used when Windows is in its dark theme, and when it is not.
const DARK_BACKGROUND: u32 = 0x0020_2020;
const DARK_TEXT: u32 = 0x00f0_f0f0;
const LIGHT_BACKGROUND: u32 = 0x00f7_f7f7;
const LIGHT_TEXT: u32 = 0x0020_2020;

struct App {
    hwnd: HWND,
    list: HWND,
    status: HWND,
    buttons: [HWND; 4],
    archive: Option<PathBuf>,
    entries: Vec<zip::Entry>,
    dark: bool,
    brush: HBRUSH,
    busy: bool,
}

impl App {
    fn background(&self) -> u32 {
        if self.dark { DARK_BACKGROUND } else { LIGHT_BACKGROUND }
    }
    fn text(&self) -> u32 {
        if self.dark { DARK_TEXT } else { LIGHT_TEXT }
    }

    /// The window is redrawn as a whole: at this size it costs nothing and it keeps the
    /// colours honest in both themes.
    fn say(&mut self, message: &str) {
        let wide = crate::wide(message);
        let _ = unsafe { SetWindowTextW(self.status, PCWSTR(wide.as_ptr())) };
    }

    /// Everything except «Открыть», which is always usable: it is the way in.
    fn allow_actions(&self, enabled: bool) {
        for button in &self.buttons[1..] {
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::EnableWindow(*button, enabled)
            };
        }
    }

    fn load(&mut self, archive: PathBuf) {
        match zip::entries(&archive) {
            Ok(entries) => {
                self.archive = Some(archive);
                self.entries = entries;
                self.fill();
                self.allow_actions(true);
                let message = format!(
                    "{} — {}, {}",
                    self.name(),
                    crate::plural(self.entries.len(), "запись", "записи", "записей"),
                    crate::size(self.entries.iter().map(|entry| entry.size).sum()),
                );
                self.say(&message);
            }
            // A name that does not exist yet is a new archive, not an error: that is how a
            // person starts one from this window.
            Err(error) => {
                if archive.exists() {
                    self.say(&format!("Не удалось открыть {}: {error}", archive.display()));
                } else {
                    self.archive = Some(archive);
                    self.entries.clear();
                    self.fill();
                    self.allow_actions(true);
                    self.say("Новый архив: добавьте в него файлы.");
                }
            }
        }
    }

    fn name(&self) -> String {
        self.archive
            .as_ref()
            .and_then(|archive| archive.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("архив не открыт"))
    }

    fn fill(&self) {
        unsafe { SendMessageW(self.list, 0x0184, None, None) }; // LB_RESETCONTENT
        for entry in &self.entries {
            let line = format!("{:>12}  {}", crate::size(entry.size), entry.name);
            let wide = crate::wide(&line);
            unsafe {
                SendMessageW(
                    self.list,
                    0x0180, // LB_ADDSTRING
                    None,
                    Some(LPARAM(wide.as_ptr() as isize)),
                )
            };
        }
    }

    fn layout(&self, width: i32, height: i32) {
        let pad = 10;
        let button_height = 28;
        let button_width = 150;
        for (index, button) in self.buttons.iter().enumerate() {
            let x = pad + index as i32 * (button_width + 6);
            let _ = unsafe { MoveWindow(*button, x, pad, button_width, button_height, BOOL(1)) };
        }
        let list_top = pad + button_height + pad;
        let status_height = 22;
        let _ = unsafe {
            MoveWindow(
                self.list,
                pad,
                list_top,
                width - pad * 2,
                height - list_top - status_height - pad,
                BOOL(1),
            )
        };
        let _ = unsafe {
            MoveWindow(
                self.status,
                pad,
                height - status_height - pad + 4,
                width - pad * 2,
                status_height,
                BOOL(1),
            )
        };
    }

    fn paint(&self) {
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(self.hwnd, &mut paint) };
        unsafe {
            let brush = CreateSolidBrush(COLORREF(self.background()));
            if let Ok(brush) = brush {
                windows::Win32::Graphics::Gdi::FillRect(dc, &paint.rcPaint, brush);
                let _ = DeleteObject(brush.into());
            }
            EndPaint(self.hwnd, &paint);
        }
    }

    fn open_dialog(&mut self) {
        let Some(files) = pick_files(self.hwnd, false, "Открыть архив") else { return };
        if let Some(archive) = files.into_iter().next() {
            self.load(archive);
        }
    }

    fn add_files(&mut self) {
        let Some(archive) = self.archive.clone() else { return };
        let Some(files) = pick_files(self.hwnd, true, "Добавить в архив") else { return };
        if files.is_empty() {
            return;
        }
        self.say("Упаковываю…");
        self.busy = true;
        let summary = zip::add(&archive, &files, false, |_, _| {});
        self.busy = false;
        match summary {
            Ok(summary) => {
                self.load(archive);
                let mut message = format!(
                    "Добавлено: {}, {}",
                    crate::plural(summary.files, "файл", "файла", "файлов"),
                    crate::size(summary.packed),
                );
                if summary.skipped > 0 {
                    message.push_str(&format!(
                        ", уже было: {}",
                        crate::plural(summary.skipped, "файл", "файла", "файлов")
                    ));
                }
                if summary.failed > 0 {
                    message.push_str(&format!(
                        ", не удалось: {}",
                        crate::plural(summary.failed, "файл", "файла", "файлов")
                    ));
                }
                self.say(&message);
            }
            Err(error) => self.say(&format!("{error}")),
        }
    }

    fn extract(&mut self) {
        let Some(archive) = self.archive.clone() else { return };
        let destination = crate::beside(&archive);
        self.say(&format!("Распаковываю в {}…", destination.display()));
        match zip::extract(&archive, &destination, |_, _| {}) {
            Ok(summary) => {
                let mut message = format!(
                    "Распаковано: {} в {}",
                    crate::plural(summary.files, "файл", "файла", "файлов"),
                    destination.display(),
                );
                if summary.failed > 0 {
                    message.push_str(&format!(
                        ", с ошибками: {}",
                        crate::plural(summary.failed, "файл", "файла", "файлов")
                    ));
                }
                self.say(&message);
            }
            Err(error) => self.say(&format!("{error}")),
        }
    }

    fn test(&mut self) {
        let Some(archive) = self.archive.clone() else { return };
        self.say("Проверяю…");
        match zip::test(&archive, |_, _| {}) {
            Ok(summary) if summary.failed == 0 => self.say(&format!(
                "Всё цело: {}",
                crate::plural(summary.files, "файл", "файла", "файлов")
            )),
            Ok(summary) => self.say(&format!(
                "Повреждено: {} из {}",
                crate::plural(summary.failed, "файл", "файла", "файлов"),
                summary.files + summary.failed
            )),
            Err(error) => self.say(&format!("{error}")),
        }
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
        // A single archive opens; anything else is added to the one already open.
        match paths.as_slice() {
            [] => {}
            [one] if is_archive(one) => self.load(one.clone()),
            _ => {
                if self.archive.is_some() {
                    let Some(archive) = self.archive.clone() else { return };
                    match zip::add(&archive, &paths, false, |_, _| {}) {
                        Ok(summary) => {
                            self.load(archive);
                            self.say(&format!(
                                "Добавлено: {}",
                                crate::plural(summary.files, "файл", "файла", "файлов")
                            ));
                        }
                        Err(error) => self.say(&format!("{error}")),
                    }
                } else if let Some(one) = paths.first() {
                    // Nothing is open: start an archive named after what was dropped.
                    let folder = one.parent().map(Path::to_path_buf).unwrap_or_default();
                    let stem = one
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_else(|| String::from("архив"));
                    let archive = folder.join(format!("{stem}.zip"));
                    match zip::add(&archive, &paths, false, |_, _| {}) {
                        Ok(_) => self.load(archive),
                        Err(error) => self.say(&format!("{error}")),
                    }
                }
            }
        }
    }
}

fn is_archive(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
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

/// The picker hands back one name, or a folder followed by the names chosen inside it.
fn claim_paths(buffer: &[u16]) -> Vec<PathBuf> {
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    let first = PathBuf::from(String::from_utf16_lossy(&buffer[..end]));
    let mut rest: Vec<PathBuf> = Vec::new();
    let mut at = end + 1;
    while at < buffer.len() && buffer[at] != 0 {
        let stop = buffer[at..].iter().position(|unit| *unit == 0).map(|offset| at + offset).unwrap_or(buffer.len());
        rest.push(PathBuf::from(String::from_utf16_lossy(&buffer[at..stop])));
        at = stop + 1;
    }
    if rest.is_empty() {
        vec![first]
    } else {
        rest.into_iter().map(|name| first.join(name)).collect()
    }
}

/// What every child control of the archiver window needs to know.
struct Parent {
    window: HWND,
    instance: HINSTANCE,
    font: HGDIOBJ,
}

impl Parent {
    fn child(&self, class: PCWSTR, text: PCWSTR, style: windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE, id: usize) -> HWND {
        // A child control carries its identifier in the menu slot: that number is what arrives
        // back in `WM_COMMAND` when the button is pressed.
        let menu = (id != 0).then(|| HMENU(id as isize));
        let child = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                text,
                style,
                0,
                0,
                0,
                0,
                Some(self.window),
                menu,
                Some(self.instance),
                None,
            )
        };
        if !child.0.is_null() {
            // The stock font is what Windows itself uses for buttons and lists.
            unsafe { SendMessageW(child, 0x0030, Some(WPARAM(self.font.0 as usize)), Some(LPARAM(1))) };
        }
        child
    }

    fn button(&self, text: PCWSTR, id: usize) -> HWND {
        self.child(w!("BUTTON"), text, WS_CHILD | WS_VISIBLE | WS_TABSTOP, id)
    }
}

fn dark_mode() -> bool {
    let mut value = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    read.0 == 0 && value == 0
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_NCDESTROY {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if !state.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let state = Box::from_raw(state);
            let _ = DeleteObject(state.brush.into());
        }
        return DefWindowProcW(hwnd, message, wp, lp);
    }
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if state.is_null() {
        return DefWindowProcW(hwnd, message, wp, lp);
    }
    let app = &mut *state;
    match message {
        WM_CREATE => LRESULT(0),
        WM_SIZE => {
            let mut rect = windows::Win32::Foundation::RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            app.layout(rect.right - rect.left, rect.bottom - rect.top);
            LRESULT(0)
        }
        WM_PAINT => {
            app.paint();
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX => {
            let dc = HDC(wp.0 as *mut _);
            let _ = SetTextColor(dc, COLORREF(app.text()));
            let _ = SetBkColor(dc, COLORREF(app.background()));
            LRESULT(app.brush.0 as isize)
        }
        WM_COMMAND => {
            if app.busy {
                return LRESULT(0);
            }
            match wp.0 & 0xffff {
                OPEN => app.open_dialog(),
                ADD => app.add_files(),
                EXTRACT => app.extract(),
                TEST => app.test(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DROPFILES => {
            if !app.busy {
                app.dropped(HDROP(lp.0 as *mut _));
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == 0x1b => {
            let _ = DestroyWindow(hwnd);
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
        let instance = GetModuleHandleW(None)
            .map(|module| HINSTANCE(module.0))
            .unwrap_or(HINSTANCE(std::ptr::null_mut()));
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR(std::ptr::null_mut())),
            lpszClassName: w!("AuraArcWindow"),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return 1;
        }
        let dark = dark_mode();
        let brush = CreateSolidBrush(COLORREF(if dark { DARK_BACKGROUND } else { LIGHT_BACKGROUND }))
            .unwrap_or(HBRUSH(std::ptr::null_mut()));
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("AuraArcWindow"),
            w!("AuraArc — архиватор AuraUI"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            720,
            520,
            None,
            None,
            Some(instance),
            None,
        );
        if window.0.is_null() {
            return 1;
        }
        let font = GetStockObject(DEFAULT_GUI_FONT);
        let panel = Parent { window, instance, font };
        let buttons = [
            panel.button(w!("Открыть архив…"), OPEN),
            panel.button(w!("Добавить файлы…"), ADD),
            panel.button(w!("Распаковать рядом"), EXTRACT),
            panel.button(w!("Проверить"), TEST),
        ];
        let list = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("LISTBOX"),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_TABSTOP | LBS_NOINTEGRALHEIGHT,
            0,
            0,
            0,
            0,
            Some(window),
            None,
            Some(instance),
            None,
        );
        let status = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Архив не открыт. Перетащите .zip на окно или нажмите «Открыть архив…»."),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            Some(window),
            None,
            Some(instance),
            None,
        );
        if !status.0.is_null() {
            SendMessageW(status, 0x0030, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        }
        if !list.0.is_null() {
            SendMessageW(list, 0x0030, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        }
        let state = Box::new(App {
            hwnd: window,
            list,
            status,
            buttons,
            archive: None,
            entries: Vec::new(),
            dark,
            brush,
            busy: false,
        });
        let pointer = Box::into_raw(state);
        SetWindowLongPtrW(window, GWLP_USERDATA, pointer as isize);
        DragAcceptFiles(window, BOOL(1));
        let app = &mut *pointer;
        app.allow_actions(false);
        app.say("Архив не открыт. Перетащите .zip на окно или нажмите «Открыть архив…».");
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
}
