//! Downloads the user-selected Pervii catalogue, only on explicit request.
//! The published endpoint is HTTP, so its contents are not authenticated.
//! Do not use this catalogue for anything except user-selected audio playback.
use crate::radio_playlist::{self, Station};
use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

struct Handle(*mut core::ffi::c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

pub fn load() -> Option<Vec<Station>> {
    unsafe {
        let session = Handle(WinHttpOpen(
            w!("AuraDock Radio"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return None;
        }
        WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 5000).ok()?;
        let conn = Handle(WinHttpConnect(session.0, w!("radio.pervii.com"), 80, 0));
        if conn.0.is_null() {
            return None;
        }
        let req = Handle(WinHttpOpenRequest(
            conn.0,
            w!("GET"),
            w!("/top_radio_russian.m3u"),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_OPEN_REQUEST_FLAGS(0),
        ));
        if req.0.is_null() {
            return None;
        }
        // Prevent an untrusted plaintext response from redirecting into a local network.
        WinHttpSetOption(
            Some(req.0),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(&WINHTTP_DISABLE_REDIRECTS.to_ne_bytes()),
        )
        .ok()?;
        WinHttpSendRequest(req.0, None, None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).ok()?;
        let mut status = 0u32;
        let mut len = 4u32;
        let mut idx = 0u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut len,
            &mut idx,
        )
        .ok()?;
        if status != 200 {
            return None;
        }
        let mut body = Vec::new();
        let mut buf = [0u8; 16384];
        loop {
            let mut got = 0u32;
            WinHttpReadData(
                req.0,
                buf.as_mut_ptr() as *mut _,
                buf.len() as u32,
                &mut got,
            )
            .ok()?;
            if got == 0 {
                break;
            }
            if body.len().saturating_add(got as usize) > radio_playlist::MAX_PLAYLIST_BYTES {
                return None;
            }
            body.extend_from_slice(&buf[..got as usize]);
        }
        let stations = radio_playlist::parse(&body).ok()?;
        if stations.is_empty() {
            None
        } else {
            Some(stations)
        }
    }
}
