//! Operating-system Explorer handoff. Fences does not embed AuraExplorer's file window.

use anyhow::{Context, Result};
use std::ffi::OsString;

pub fn open_system_explorer(target: OsString) -> Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(target)
        .spawn()
        .context("could not start the Windows file manager")?;
    Ok(())
}
