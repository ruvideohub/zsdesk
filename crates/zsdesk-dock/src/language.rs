//! UI language for the dock.
//!
//! Follows the same convention as the file manager: a profile language
//! preference wins, `ZSDESK_UI_LANGUAGE` is inherited from ZSDesk when the
//! dock is started from its menu, and non-Chinese Windows systems use the
//! translation catalog (Russian in this fork).

use windows::Win32::Globalization::GetUserDefaultUILanguage;

/// Overrides injected by the file manager when it spawns the dock.
pub fn initialize() {
    let enabled = match std::env::var("ZSDESK_UI_LANGUAGE").as_deref() {
        Ok("en") => true,
        Ok("zh-CN") => false,
        _ => {
            let language = unsafe { GetUserDefaultUILanguage() };
            language & 0x3ff != 0x04
        }
    };
    zsdesk_core::i18n::set_english(enabled);
}
