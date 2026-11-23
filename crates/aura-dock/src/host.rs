//! The desktop-widget host: the part of `widgets.exe` that draws on the desktop.
//!
//! Two widgets live here: the desktop clock card and the desktop load card (processor,
//! memory, disk). Each is a window of its own process — `widgets.exe --component clock` and
//! `widgets.exe --component system` — so a redraw problem or a hang on the desktop cannot
//! take the panel, and the taskbar the panel stands in for, down with it.
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
use crate::desktop_system::DesktopSystem;
use crate::gfx::Gfx;

/// How often the desktop is checked for the panel still being alive.
const PARENT_CHECK: Duration = Duration::from_millis(1000);

/// What both desktop cards give the message loop.
trait Card {
    fn hwnd(&self) -> HWND;
    fn repaint(&mut self, c: &Compositor, gfx: &Gfx) -> Result<()>;
    fn tick(&mut self, c: &Compositor, gfx: &Gfx) -> Result<()>;
}

impl Card for DesktopClock {
    fn hwnd(&self) -> HWND { DesktopClock::hwnd(self) }
    fn repaint(&mut self, c: &Compositor, gfx: &Gfx) -> Result<()> { DesktopClock::repaint(self, c, gfx) }
    fn tick(&mut self, c: &Compositor, gfx: &Gfx) -> Result<()> { DesktopClock::tick(self, c, gfx) }
}

impl Card for DesktopSystem {
    fn hwnd(&self) -> HWND { DesktopSystem::hwnd(self) }
    fn repaint(&mut self, c: &Compositor, gfx: &Gfx) -> Result<()> { DesktopSystem::repaint(self, c, gfx) }
    fn tick(&mut self, c: &Compositor, gfx: &Gfx) -> Result<()> { DesktopSystem::tick(self, c, gfx) }
}

/// The one-second message loop both cards run in: draw, keep the card under the app in front,
/// and quit when the panel that owns it is gone.
fn pump<C: Card>(dock: HWND, mut card: C, compositor: &Compositor, gfx: &Gfx) -> Result<()> {
    card.repaint(compositor, gfx)?;
    let window = card.hwnd();
    unsafe {
        SetTimer(Some(window), TIMER_TICK, 1000, None);
    }
    let mut msg = MSG::default();
    let mut seen = Instant::now();
    loop {
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !got.as_bool() {
            break;
        }
        // Timer-only traffic is the normal idle case; check before the timer's
        // early continue so a dead parent cannot leave an orphaned card behind.
        if seen.elapsed() >= PARENT_CHECK {
            seen = Instant::now();
            if !unsafe { IsWindow(Some(dock)) }.as_bool() { break; }
        }
        if msg.message == WM_TIMER && msg.hwnd == window {
            let _ = card.tick(compositor, gfx);
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// The display, the theme and the compositor: the same for every desktop card.
fn prepare(dock: Option<HWND>) -> Result<Option<(Compositor, Gfx, windows::Win32::Foundation::RECT, f32, HWND)>> {
    let Some(dock) = dock else { return Ok(None) };
    unsafe {
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    crate::dpi_aware();
    crate::composition_apartment()?;
    let cfg = crate::config::Config::load().unwrap_or_default();
    let light = match std::env::var("LAMINAR_THEME").ok().as_deref() {
        Some("light") => true,
        Some("dark") => false,
        _ => cfg.match_windows_theme && !laminar_sys::theme::apps_use_dark(),
    };
    crate::ui::theme::set_light(light);
    let (monitor, dpi) = match crate::displays::all().first().copied() {
        Some(d) => (d.rect, d.dpi),
        None => {
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
    Ok(Some((compositor, gfx, monitor, dpi as f32 / 96.0, dock)))
}

/// Draw the desktop load card until the panel it belongs to is gone.
pub fn run_system(dock: Option<HWND>) -> Result<()> {
    let Some((compositor, gfx, monitor, dpi, dock)) = prepare(dock)? else {
        eprintln!("widgets: no --dock <hwnd>: the desktop load card needs the panel it reports to");
        return Ok(());
    };
    let cfg = crate::config::Config::load().unwrap_or_default();
    let prefs = crate::desktop_system::Prefs::from_config(&cfg);
    let card = DesktopSystem::new(&compositor, monitor, dpi, prefs, dock)?;
    pump(dock, card, &compositor, &gfx)
}

/// Draw the desktop clock until the panel it belongs to is gone.
///
/// `dock` is the panel's window: the card posts its menu and drag requests there, and the
/// process quits as soon as that window disappears. `None` (no panel handle on the command
/// line) still draws the card — it is what a bare `widgets.exe` does — but then nothing can
/// change its settings, so it also quits at once with a note.
pub fn run_clock(dock: Option<HWND>) -> Result<()> {
    let Some((compositor, gfx, monitor, dpi, dock)) = prepare(dock)? else {
        eprintln!("widgets: no --dock <hwnd>: the desktop clock needs the panel it reports to");
        return Ok(());
    };
    let cfg = crate::config::Config::load().unwrap_or_default();
    let prefs = Prefs::from_config(&cfg);
    let clock = DesktopClock::new(&compositor, monitor, dpi, prefs, dock)?;
    pump(dock, clock, &compositor, &gfx)
}

/// Timer id for the card's one-second refresh.
const TIMER_TICK: usize = 1;
