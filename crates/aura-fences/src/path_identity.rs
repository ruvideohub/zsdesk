//! Path identity helper for Shell results; this module does not forward to AuraExplorer.

use std::path::Path;

pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let units = |path: &Path| path.as_os_str().encode_wide().map(|value| if value == 47 { 92 } else { value }).collect::<Vec<_>>();
    let mut left = units(a);
    let mut right = units(b);
    while left.len() > 3 && left.last() == Some(&92) { left.pop(); }
    while right.len() > 3 && right.last() == Some(&92) { right.pop(); }
    (unsafe { windows::Win32::Globalization::CompareStringOrdinal(&left, &right, true) })
        == windows::Win32::Globalization::CSTR_EQUAL
}
