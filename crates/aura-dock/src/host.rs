//! The desktop-widget host: the part of `widgets.exe` that draws on the desktop.
//!
//! The first widget is the desktop clock card. It used to be a window created *inside the
//! panel's process*; it lives here now, in a process of its own, so a redraw problem or a
//! hang on the desktop cannot take the panel — and the taskbar the panel stands in for —
//! down with it.
//!
//! What this process does **not** do is write settings. It reads `config::Config` once, and
//! when the card is dragged or right-clicked it posts the same messages the card has always
//! posted (`WM_APP_CLOCK_MOVED`, `WM_APP_CLOCK_MENU`) to the panel's window — which may now
//! be in another process, hence the window handle on the command line. The panel saves and
//! restarts this process when the settings change, so there is still exactly one writer.

use std::time::{Duration, Instant};

use windows::core::Result;
use windows::UI::Composition::Compositor;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, IsWindow, SetTimer, TranslateMessage, MSG, WM_TIMER,
};

use crate::desktop_clock::{DesktopClock, Prefs};
use crate::gfx::Gfx;

/// How often the desktop is checked for the panel still being alive.
const PARENT_CHECK: Duration = Duration::from_millis(1000);

/// Draw the desktop clock until the panel it belongs to is gone.
///
/// `dock` is the panel's window: the card posts its menu and drag requests there, and the
/// process quits as soon as that window disappears. `None` (no panel handle on the command
/// line) still draws the card — it is what a bare `widgets.exe` does — but then nothing can
/// change its settings, so it also quits at once with a note.
pub fn run_clock(dock: Option<HWND>) -> Result<()> {
    let Some(dock) = dock else {
        eprintln!("widgets: no --dock <hwnd>: the desktop clock needs the panel it reports to");
        return Ok(());
    };
    unsafe {
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    crate::dpi_aware();
    crate::composition_apartment()?;

    let cfg = crate::config::Config::load().unwrap_or_default();
    // The card follows the same light/dark decision the panel makes.
    let light = match std::env::var("LAMINAR_THEME").ok().as_deref() {
        Some("light") => true,
        Some("dark") => false,
        _ => cfg.match_windows_theme && !laminar_sys::theme::apps_use_dark(),
    };
    crate::ui::theme::set_light(light);
    let prefs = Prefs::from_config(&cfg);
    // The card lives on the primary display, the way its corner setting is written.
    let (monitor, dpi) = match crate::displays::all().first().copied() {
        Some(d) => (d.rect, d.dpi),
        None => {
            // No display enumeration at all: the screen itself is the best guess we have.
            let (x, y, w, h) = unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{
                    GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
                };
                (
                    GetSystemMetrics(SM_XVIRTUALSCREEN),
                    GetSystemMetrics(SM_YVIRTUALSCREEN),
                    GetSystemMetrics(SM_CXSCREEN).max(1),
                    GetSystemMetrics(SM_CYSCREEN).max(1),
                )
            };
            (windows::Win32::Foundation::RECT { left: x, top: y, right: x + w, bottom: y + h }, 96)
        }
    };
    let compositor = Compositor::new()?;
    let gfx = Gfx::new(&compositor)?;
    let mut clock = DesktopClock::new(&compositor, monitor, dpi as f32 / 96.0, prefs, dock)?;
    clock.repaint(&compositor, &gfx)?;
    let card = clock.hwnd();
    unsafe {
        SetTimer(Some(card), TIMER_TICK, 1000, None);
    }

    let mut msg = MSG::default();
    let mut seen = Instant::now();
    loop {
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !got.as_bool() {
            break;
        }
        if msg.message == WM_TIMER && msg.hwnd == card {
            let _ = clock.tick(&compositor, &gfx);
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if seen.elapsed() >= PARENT_CHECK {
            seen = Instant::now();
            if !unsafe { IsWindow(Some(dock)) }.as_bool() {
                // The panel is gone: so is the reason to draw its clock.
                break;
            }
        }
    }
    Ok(())
}

/// Timer id for the card's one-second refresh.
const TIMER_TICK: usize = 1;
