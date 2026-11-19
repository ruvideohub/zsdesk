//! Night light and the notifications (Do Not Disturb) switch.
//!
//! **Night light.** Windows keeps its night-light schedule behind an internal API, so this
//! module does what f.lux and friends do: it writes a warm gamma ramp for the desktop with
//! `SetDeviceGammaRamp`. The user's own ramp is read once with `GetDeviceGammaRamp` and put
//! back when night light is turned off, so switching it off restores the screen exactly.
//!
//! **Notifications.** `NOC_GLOBAL_SETTING_TOASTS_ENABLED` under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings` is the switch
//! Windows' own Settings app writes for "turn notifications off"; toast hosts honour it.
//!
//! No COM needed, both calls return in a few milliseconds.

use windows::core::{s, w};
use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC, HDC};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

/// `GetDeviceGammaRamp`/`SetDeviceGammaRamp` are gdi32 exports; the metadata doesn't put
/// them in a module this crate enables, so they are looked up at run time.
type Ramp = unsafe extern "system" fn(HDC, *mut core::ffi::c_void) -> i32;

fn gdi(name: windows::core::PCSTR) -> Option<Ramp> {
    unsafe {
        let module = GetModuleHandleW(w!("gdi32.dll")).ok()?;
        let proc = GetProcAddress(module, name)?;
        Some(std::mem::transmute_copy(&proc))
    }
}

const RAMP_LEN: usize = 256;

/// The plain Windows ramp (what a monitor without any correction uses).
fn identity() -> [u16; 3 * RAMP_LEN] {
    let mut out = [0u16; 3 * RAMP_LEN];
    for i in 0..RAMP_LEN {
        let v = (i * 257) as u16;
        out[i] = v;
        out[RAMP_LEN + i] = v;
        out[2 * RAMP_LEN + i] = v;
    }
    out
}

/// The desktop's own ramp, as it is right now (falls back to the identity ramp).
fn current() -> [u16; 3 * RAMP_LEN] {
    let mut ramp = identity();
    unsafe {
        if let (Some(get), dc) = (gdi(s!("GetDeviceGammaRamp")), GetDC(None)) {
            if !dc.is_invalid() {
                let _ = get(dc, ramp.as_mut_ptr() as *mut core::ffi::c_void);
                ReleaseDC(None, dc);
            }
        }
    }
    ramp
}

/// A warm ramp: green down a little, blue down a lot, red untouched.
fn warm(strength: f32) -> [u16; 3 * RAMP_LEN] {
    let s = strength.clamp(0.0, 1.0);
    let mut ramp = identity();
    let (g, b) = (1.0 - 0.20 * s, 1.0 - 0.42 * s);
    for i in 0..RAMP_LEN {
        ramp[RAMP_LEN + i] = (ramp[RAMP_LEN + i] as f32 * g) as u16;
        ramp[2 * RAMP_LEN + i] = (ramp[2 * RAMP_LEN + i] as f32 * b) as u16;
    }
    ramp
}

fn apply(ramp: &mut [u16; 3 * RAMP_LEN]) -> bool {
    let Some(set) = gdi(s!("SetDeviceGammaRamp")) else { return false };
    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return false;
        }
        let ok = set(dc, ramp.as_mut_ptr() as *mut core::ffi::c_void) != 0;
        ReleaseDC(None, dc);
        ok
    }
}

/// The ramp to restore when night light goes off, remembered the first time it is turned on.
fn remembered() -> &'static std::sync::Mutex<Option<[u16; 3 * RAMP_LEN]>> {
    static R: std::sync::OnceLock<std::sync::Mutex<Option<[u16; 3 * RAMP_LEN]>>> = std::sync::OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(None))
}

/// Turns night light on (`strength` 0..1 of warmth, 0.6 is comfortable) or off again.
pub fn set_night_light(on: bool, strength: f32) -> bool {
    let mut remembered = remembered().lock().unwrap_or_else(|e| e.into_inner());
    if on {
        if remembered.is_none() {
            *remembered = Some(current());
        }
        apply(&mut warm(strength))
    } else {
        let mut back = remembered.take().unwrap_or_else(identity);
        apply(&mut back)
    }
}

const NOTIFICATIONS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings";
const TOASTS: &str = "NOC_GLOBAL_SETTING_TOASTS_ENABLED";

/// Whether notification banners are allowed (Windows' own "notifications" switch).
pub fn notifications_enabled() -> Option<bool> {
    crate::theme::read_dword(NOTIFICATIONS, TOASTS).map(|v| v != 0)
}

/// Turns notification banners off (`false`, do not disturb) or back on.
pub fn set_notifications_enabled(on: bool) -> bool {
    crate::theme::write_dword(NOTIFICATIONS, TOASTS, if on { 1 } else { 0 })
}
