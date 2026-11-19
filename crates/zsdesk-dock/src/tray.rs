//! Live values behind the status sector: network, battery and master volume.
//!
//! The dock is not the shell, so it cannot own the notification area — see
//! `docs/dock.md`. What it can do is read the few values Windows exposes
//! publicly (`InternetGetConnectedState`, `GetSystemPowerStatus`, Core Audio)
//! and show them next to the clock. Reading only: clicks open the system
//! flyouts, they never move another application's tray icon.

/// One sample of the values the sector shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// A connection with internet access is available.
    pub online: bool,
    /// Battery charge in percent; `None` when the machine has no battery.
    pub battery: Option<u8>,
    /// The machine runs on mains power.
    pub charging: bool,
    /// Master volume in percent; `None` when there is no audio endpoint.
    pub volume: Option<u8>,
    /// The output is muted.
    pub muted: bool,
}

impl Snapshot {
    /// Battery charge to show, or `None` when the value is unknown.
    ///
    /// Windows reports `255` for "no battery or unknown"; that must not turn
    /// into a full battery icon.
    pub fn battery_level(raw: u8) -> Option<u8> {
        match raw {
            0..=100 => Some(raw),
            _ => None,
        }
    }

    /// Volume rounded from the 0..1 endpoint scale, clamped to 0..100.
    pub fn volume_level(scalar: f32) -> u8 {
        if scalar.is_finite() {
            (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
        } else {
            0
        }
    }
}

#[cfg(windows)]
pub use imp::query;

#[cfg(not(windows))]
pub fn query() -> Snapshot {
    Snapshot::default()
}

/// Sets the master volume; `false` when there is no audio endpoint or the
/// platform has no Core Audio.
pub fn set_volume(percent: u8) -> bool {
    #[cfg(windows)]
    { imp::set_volume(percent) }
    #[cfg(not(windows))]
    { let _ = percent; false }
}

/// Mutes or unmutes the default output device.
pub fn set_muted(muted: bool) -> bool {
    #[cfg(windows)]
    { imp::set_muted(muted) }
    #[cfg(not(windows))]
    { let _ = muted; false }
}

/// Opens the Windows quick settings flyout (`Win+A`), the closest thing to a
/// replacement for the notification area that the public API allows.
pub fn quick_settings() { press_win_key('A') }

/// Opens the notification centre (`Win+N`).
pub fn notifications() { press_win_key('N') }

#[cfg(windows)]
fn press_win_key(letter: char) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_LWIN,
    };
    let Ok(letter) = u16::try_from(letter as u32).map(|code| VIRTUAL_KEY(code)) else { return };
    let key = |code: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            ki: KEYBDINPUT { wVk: code, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    };
    let sequence = [
        key(VK_LWIN, KEYBD_EVENT_FLAGS(0)),
        key(letter, KEYBD_EVENT_FLAGS(0)),
        key(letter, KEYEVENTF_KEYUP),
        key(VK_LWIN, KEYEVENTF_KEYUP),
    ];
    unsafe { SendInput(&sequence, std::mem::size_of::<INPUT>() as i32) };
}

#[cfg(not(windows))]
fn press_win_key(_letter: char) {}

#[cfg(windows)]
mod imp {
    use super::Snapshot;

    use windows::Win32::{
        Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator},
        Media::Audio::Endpoints::IAudioEndpointVolume,
        Networking::WinInet::{InternetGetConnectedState, INTERNET_CONNECTION},
        System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED},
        System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
    };

    /// Reads every value once. Cheap enough for the one-second dock refresh:
    /// the audio endpoint is created on first use and then reused.
    pub fn query() -> Snapshot {
        let mut snapshot = Snapshot { online: online(), ..Default::default() };
        let (battery, charging) = power();
        snapshot.battery = battery;
        snapshot.charging = charging;
        if let Some((level, muted)) = volume() {
            snapshot.volume = Some(level);
            snapshot.muted = muted;
        }
        snapshot
    }

    fn online() -> bool {
        let mut flags = INTERNET_CONNECTION(0);
        // The wrapper turns a FALSE return into an error, so `Ok` means online.
        unsafe { InternetGetConnectedState(&mut flags, None) }.is_ok()
    }

    /// `(charge, on mains)`; the charge is `None` on desktops.
    fn power() -> (Option<u8>, bool) {
        let mut status = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
            return (None, false);
        }
        // ACLineStatus: 0 = battery, 1 = mains, 255 = unknown.
        let charging = status.ACLineStatus == 1;
        (Snapshot::battery_level(status.BatteryLifePercent), charging)
    }

    /// `(percent, muted)` of the default output device.
    fn volume() -> Option<(u8, bool)> {
        endpoint(|endpoint| {
            let level = unsafe { endpoint.GetMasterVolumeLevelScalar() }.ok()?;
            let muted = unsafe { endpoint.GetMute() }.ok()?.as_bool();
            Some((Snapshot::volume_level(level), muted))
        })
    }

    /// Runs `action` on the default output endpoint, creating it on first use.
    fn endpoint<T>(action: impl FnOnce(&IAudioEndpointVolume) -> Option<T>) -> Option<T> {
        ENDPOINT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                // The dock thread owns the apartment for the whole process
                // lifetime; a changed mode means somebody initialised COM with
                // another model first, which is fine for the master volume.
                let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
                let enumerator: IMMDeviceEnumerator =
                    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
                let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
                let endpoint: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }.ok()?;
                *slot = Some(endpoint);
            }
            action(slot.as_ref()?)
        })
    }

    /// Sets the master volume, `0..=100`.
    pub fn set_volume(percent: u8) -> bool {
        let level = percent.min(100) as f32 / 100.0;
        endpoint(|endpoint| unsafe { endpoint.SetMasterVolumeLevelScalar(level, std::ptr::null()) }.ok()).is_some()
    }

    /// Mutes or unmutes the default output device.
    pub fn set_muted(muted: bool) -> bool {
        endpoint(|endpoint| unsafe { endpoint.SetMute(muted, std::ptr::null()) }.ok()).is_some()
    }

    thread_local! {
        static ENDPOINT: std::cell::RefCell<Option<IAudioEndpointVolume>> = const { std::cell::RefCell::new(None) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_battery_value_is_not_a_full_battery() {
        assert_eq!(Snapshot::battery_level(255), None);
        assert_eq!(Snapshot::battery_level(0), Some(0));
        assert_eq!(Snapshot::battery_level(100), Some(100));
        assert_eq!(Snapshot::battery_level(101), None);
    }

    #[test]
    fn the_volume_scale_rounds_and_clamps() {
        assert_eq!(Snapshot::volume_level(0.0), 0);
        assert_eq!(Snapshot::volume_level(0.424), 42);
        assert_eq!(Snapshot::volume_level(1.0), 100);
        assert_eq!(Snapshot::volume_level(1.7), 100);
        assert_eq!(Snapshot::volume_level(f32::NAN), 0);
    }

    #[test]
    fn the_default_sample_is_silent_and_offline() {
        let snapshot = Snapshot::default();
        assert!(!snapshot.online);
        assert_eq!(snapshot.battery, None);
        assert_eq!(snapshot.volume, None);
    }
}
