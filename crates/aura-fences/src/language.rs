//! Fences currently follows the product's shipped English UI catalog.

pub fn initialize_environment() {
    aura_core::i18n::set_english(true);
    std::env::set_var("AURA_UI_LANGUAGE", "en");
}

pub const CHANGED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 219;
