//! Serializes one profile's host lifetime and its watchdog's recovery.
//!
//! The host owns this mutex on its desktop thread. The OS abandons it if that
//! thread dies. A guard waits for the host lifetime handle first, then acquires
//! it before reading or changing the profile or Explorer's icon state. A new
//! host must acquire it before loading the profile, so an older guard cannot
//! rewrite a new host's configuration while that host is running.
use std::path::Path;
use anyhow::{bail, Result};
use windows::{core::PCWSTR, Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0},
    System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
}};

pub(super) struct RecoveryLock(HANDLE);

impl RecoveryLock {
    pub(super) fn acquire(config: &Path, timeout_ms: u32) -> Result<Self> {
        // Use the same profile identity as the existing single-instance guard,
        // but a distinct kernel object: this one is an acquired mutex, not a
        // mere existence marker.
        let mut name = super::instance_key(config);
        name.pop();
        name.extend(".Recovery".encode_utf16());
        name.push(0);
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr()))? };
        let result = unsafe { WaitForSingleObject(handle, timeout_ms) };
        if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
            let _ = unsafe { CloseHandle(handle) };
            bail!("Fences profile recovery lock unavailable (wait result {:#x})", result.0);
        }
        Ok(Self(handle))
    }
}

impl Drop for RecoveryLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn same_profile_waits_and_distinct_profile_does_not() {
        let root = std::env::temp_dir().join(format!("aura-recovery-lock-{}", std::process::id()));
        let path = root.join("desktop-fences.json");
        let other = root.join("other.json");
        let host = RecoveryLock::acquire(&path, 1000).unwrap();
        let (tx, rx) = mpsc::channel();
        let path2 = path.clone();
        let worker = std::thread::spawn(move || {
            let independent = RecoveryLock::acquire(&other, 1000).is_ok();
            let blocked = RecoveryLock::acquire(&path2, 150).is_err();
            tx.send((independent, blocked)).unwrap();
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), (true, true));
        worker.join().unwrap();
        drop(host);
        assert!(std::thread::spawn(move || RecoveryLock::acquire(&path, 1000).is_ok()).join().unwrap());
    }
}
