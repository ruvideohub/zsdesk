//! Application icon loading.
//!
//! The Shell hands out icons through `IShellItemImageFactory`, which follows
//! shortcuts, resolves `.exe` resources and returns a high-resolution bitmap
//! instead of the 16/32 px `HICON` that `SHGetFileInfo` offers. Loading is
//! done on a worker thread: a cold icon (a network shortcut, a Store package)
//! can take tens of milliseconds, and the dock must never stall its own
//! animation because of it.

use std::{
    collections::HashMap,
    sync::{mpsc::{self, Sender}, Arc, Mutex},
    thread,
};

use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, SIZE, WPARAM},
        Graphics::{Direct2D::ID2D1Bitmap, Imaging::IWICBitmapSource},
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::{
            Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY},
            WindowsAndMessaging::PostMessageW,
        },
    },
};

use zsdesk_windows::gfx::{BitmapAlpha, Gfx};

/// A bitmap ready for drawing, or a request that is still in flight.
#[derive(Default)]
struct Entry {
    bitmap: Option<ID2D1Bitmap>,
    requested: bool,
}

/// Icon bitmaps keyed by the shell path they were loaded from.
#[derive(Default)]
pub struct IconCache {
    entries: HashMap<String, Entry>,
}

impl IconCache {
    pub fn bitmap(&self, key: &str) -> Option<&ID2D1Bitmap> {
        self.entries.get(key)?.bitmap.as_ref()
    }

    /// Marks the icon as requested; returns `true` when the caller should
    /// queue a load, i.e. it was neither loaded nor requested before.
    pub fn want(&mut self, key: &str) -> bool {
        let entry = self.entries.entry(key.to_string()).or_default();
        if entry.bitmap.is_some() || entry.requested { return false; }
        entry.requested = true;
        true
    }

    /// Replaces a requested icon; a failed load clears the request flag so a
    /// later pointer move may retry.
    pub fn insert(&mut self, key: &str, bitmap: Option<ID2D1Bitmap>) {
        let entry = self.entries.entry(key.to_string()).or_default();
        entry.requested = bitmap.is_none();
        entry.bitmap = bitmap;
    }

}

/// Handed from the worker thread to the window thread.
struct Loaded {
    key: String,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
}

// The GDI bitmap is owned exclusively by the queue until the window thread
// converts it and deletes it, so moving it across the channel is sound.
unsafe impl Send for Loaded {}

/// Worker thread that turns paths into `HBITMAP`s.
pub struct IconLoader {
    requests: Sender<(String, String, i32)>,
    ready: Arc<Mutex<Vec<Loaded>>>,
    notify: HWND,
    worker: Option<thread::JoinHandle<()>>,
    closed: bool,
}

impl IconLoader {
    /// `notify` receives `message` (a `WM_APP` value) after every batch.
    pub fn spawn(notify: HWND, message: u32) -> Self {
        let (requests, queue) = mpsc::channel::<(String, String, i32)>();
        let ready = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&ready);
        // The notification target travels as an address: `HWND` wraps a pointer
        // and is therefore not `Send`. `notify` is an owned window of the
        // caller that outlives the worker.
        let notify_address = notify.0 as usize;
        let worker = thread::Builder::new().name("zsdesk-dock-icons".into()).spawn(move || {
            let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            while let Ok((key, path, pixels)) = queue.recv() {
                let bitmap = load(&path, pixels);
                let Some(bitmap) = bitmap else { continue };
                if let Ok(mut list) = shared.lock() { list.push(Loaded { key, bitmap }); }
                let notify = HWND(notify_address as *mut _);
                unsafe { let _ = PostMessageW(Some(notify), message, WPARAM(0), LPARAM(0)); }
            }
            if initialized { unsafe { CoUninitialize() }; }
        }).ok();
        Self { requests, ready, notify, worker, closed: false }
    }

    pub fn request(&self, key: &str, path: &str, pixels: i32) {
        if self.closed { return; }
        let _ = self.requests.send((key.to_string(), path.to_string(), pixels.max(16)));
    }

    /// Uploads every bitmap that finished since the last call.
    pub fn drain(&self, gfx: &Gfx, cache: &mut IconCache) -> usize {
        let list = match self.ready.lock() { Ok(mut list) => std::mem::take(&mut *list), Err(_) => return 0 };
        let mut uploaded = 0;
        for Loaded { key, bitmap } in list {
            let source: Option<IWICBitmapSource> = gfx.wic_from_hbitmap(bitmap, BitmapAlpha::Premultiplied);
            let upload = source.as_ref().and_then(|source| gfx.upload(source));
            // The WIC bitmap is materialized, so GDI's copy can go immediately.
            let _ = unsafe { windows::Win32::Graphics::Gdi::DeleteObject(bitmap.into()) };
            if upload.is_some() { uploaded += 1; }
            cache.insert(&key, upload);
        }
        uploaded
    }

    pub fn shutdown(&mut self) {
        self.closed = true;
        // Dropping the sender ends the worker's `recv` loop.
        let (closed, _) = mpsc::channel::<(String, String, i32)>();
        let replacement = closed;
        let old = std::mem::replace(&mut self.requests, replacement);
        drop(old);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for IconLoader {
    fn drop(&mut self) {
        self.closed = true;
        let _ = self.notify;
    }
}

/// Extracts one icon. Returns `None` for paths the Shell cannot resolve.
fn load(path: &str, pixels: i32) -> Option<windows::Win32::Graphics::Gdi::HBITMAP> {
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }.ok()?;
    let size = SIZE { cx: pixels, cy: pixels };
    unsafe { factory.GetImage(size, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK) }.ok()
}
