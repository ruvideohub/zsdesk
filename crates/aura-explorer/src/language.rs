//! Интерфейс — по-русски, всегда.
//!
//! Русский каталог лежит в паке, который в коде называется `English`
//! (`crates/aura-core/resources/strings.en.json` переводит китайские исходные строки).
//! Выбора языка больше нет: на русской Windows режим «Система» давал английский, на
//! китайской — китайский, а переключатель назывался «English» и выглядел как ошибка.
//!
//! Прежние `choice` / `save_and_apply` / `English`/`Chinese`-ветки убраны; сохранённое
//! в профиле `ui.language` читается как раньше (поле осталось, чтобы старые профили
//! открывались), но ни на что не влияет.
use std::path::Path;
pub use aura_core::i18n::LanguagePreference;

/// Включить русский каталог. Единственная точка, где это решается.
pub fn apply(_preference: LanguagePreference) {
    aura_core::i18n::set_english(true);
    // Дочерние процессы (меню в отдельном процессе, панель) читают язык из окружения.
    std::env::set_var("AURA_UI_LANGUAGE", "en");
}

pub fn initialize_environment() {
    apply(LanguagePreference::English);
}

pub fn initialize_profile(_path: &Path) {
    apply(LanguagePreference::English);
}

pub const CHANGED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 219;

#[allow(dead_code)]
pub fn notify_windows() {
    use windows::core::BOOL;
    use windows::Win32::{Foundation::{HWND, LPARAM, WPARAM}, UI::WindowsAndMessaging::*};
    unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)); }
        if pid == std::process::id() {
            let mut buf = [0u16; 64];
            let n = unsafe { GetClassNameW(hwnd, &mut buf) }.max(0) as usize;
            if matches!(String::from_utf16_lossy(&buf[..n]).as_str(),
                "AuraExplorer.Native.Explorer" | "AuraExplorer.UtilityPanel" | "AuraExplorer.Native.DesktopHost") {
                let _ = unsafe { PostMessageW(Some(hwnd), CHANGED, WPARAM(0), LPARAM(0)) };
            }
        }
        true.into()
    }
    let _ = unsafe { EnumWindows(Some(each), LPARAM(0)) };
    crate::runtime::refresh_tray();
}
