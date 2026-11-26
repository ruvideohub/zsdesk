//! OS appearance preferences for the independent desktop-widget processes.
use crate::desktop_card_policy::Environment;
use windows::Win32::UI::{
    Accessibility::HIGHCONTRASTW,
    WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    },
};

pub fn read(match_windows_theme: bool) -> Environment {
    let light = match std::env::var("LAMINAR_THEME").ok().as_deref() {
        Some("light") => true,
        Some("dark") => false,
        _ => match_windows_theme && !laminar_sys::theme::apps_use_dark(),
    };
    let mut contrast = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    let high_contrast = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok()
        && contrast.dwFlags.0 & 1 != 0;
    let system_colors = if high_contrast {
        unsafe {
            use windows::Win32::Graphics::Gdi::{
                GetSysColor, COLOR_HIGHLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT,
            };
            [
                GetSysColor(COLOR_WINDOW),
                GetSysColor(COLOR_WINDOWTEXT),
                GetSysColor(COLOR_HIGHLIGHT),
            ]
        }
    } else {
        [0; 3]
    };
    Environment {
        light,
        high_contrast,
        transparency: laminar_sys::theme::transparency_enabled(),
        system_colors,
    }
}
pub fn apply(env: Environment) {
    crate::ui::theme::set_light(env.light);
    crate::ui::theme::set_desktop_accessibility(env.high_contrast, env.transparency);
}
