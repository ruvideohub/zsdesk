//! Minimal standalone diagnostics for the Fences host.

pub fn trace(message: &str) {
    let text: Vec<u16> = format!("Fences: {message}\r\n").encode_utf16().chain(Some(0)).collect();
    unsafe { windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(windows::core::PCWSTR(text.as_ptr())); }
}
