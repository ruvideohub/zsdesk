//! Small archive-name helpers for Fences' standalone context menu.

use std::path::PathBuf;

/// The first `.zip` among the selected items, if there is one.
pub(crate) fn archive_among(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))).cloned()
}
