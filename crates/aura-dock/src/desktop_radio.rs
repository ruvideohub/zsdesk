//! Small, bounded IPC between the desktop card and the Dock's single radio player.
use serde::{Deserialize, Serialize};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::DataExchange::COPYDATASTRUCT,
    UI::WindowsAndMessaging::{PostMessageW, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_COPYDATA},
};

const MAGIC: usize = 0x4155_5244; // AURD
const MAX_BYTES: usize = 16 * 1024;
pub const QUERY: isize = 0;
pub const PREVIOUS: isize = 1;
pub const TOGGLE: isize = 2;
pub const NEXT: isize = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct View { pub name: String, pub caption: String, pub playing: bool }

pub fn request(dock: HWND, card: HWND, action: isize) {
    unsafe { let _ = PostMessageW(Some(dock), crate::dock::WM_APP_RADIO_REQUEST, WPARAM(card.0 as usize), LPARAM(action)); }
}

/// The caller validates the recipient PID before calling this. Timeout bounds UI stalls.
pub fn send_view(card: HWND, dock: HWND, view: &View) {
    let Ok(bytes) = serde_json::to_vec(view) else { return };
    if bytes.len() > MAX_BYTES { return; }
    let data = COPYDATASTRUCT { dwData: MAGIC, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
    unsafe { let _ = SendMessageTimeoutW(card, WM_COPYDATA, WPARAM(dock.0 as usize), LPARAM(&data as *const _ as isize), SMTO_ABORTIFHUNG, 100, None); }
}

/// Only call from a WM_COPYDATA handler after checking the sender window. The OS supplies
/// the cross-process buffer for the lifetime of the synchronous message.
pub unsafe fn receive(ptr: isize) -> Option<View> {
    if ptr == 0 { return None; }
    let data = unsafe { &*(ptr as *const COPYDATASTRUCT) };
    if data.dwData != MAGIC || data.cbData as usize > MAX_BYTES || data.lpData.is_null() { return None; }
    let bytes = unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
    serde_json::from_slice(bytes).ok()
}

/// Buttons in logical pixels. The rest of the card remains draggable.
pub fn hit(x: f32, y: f32) -> Option<isize> {
    if !(117.0..147.0).contains(&y) { return None; }
    if (52.0..96.0).contains(&x) { Some(PREVIOUS) }
    else if (112.0..156.0).contains(&x) { Some(TOGGLE) }
    else if (172.0..216.0).contains(&x) { Some(NEXT) }
    else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buttons_never_steal_drag_space() {
        assert_eq!(hit(52.0, 117.0), Some(PREVIOUS));
        assert_eq!(hit(134.0, 130.0), Some(TOGGLE));
        assert_eq!(hit(200.0, 130.0), Some(NEXT));
        assert_eq!(hit(96.0, 130.0), None);
        assert_eq!(hit(134.0, 147.0), None);
        assert_eq!(hit(f32::NAN, 130.0), None);
    }
    #[test]
    fn view_round_trip_and_payload_validation() {
        let view = View { name: "Радио".into(), caption: "Paused".into(), playing: false };
        let bytes = serde_json::to_vec(&view).unwrap();
        let mut data = COPYDATASTRUCT { dwData: MAGIC, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
        assert_eq!(unsafe { receive(&data as *const _ as isize) }, Some(view));
        data.dwData = 0;
        assert!(unsafe { receive(&data as *const _ as isize) }.is_none());
        data.dwData = MAGIC;
        data.cbData = (MAX_BYTES + 1) as u32;
        assert!(unsafe { receive(&data as *const _ as isize) }.is_none());
    }
}
