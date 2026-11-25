//! Safe ownership rules for Dock's per-user login entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change { None, Write, Delete, Conflict }

/// A quote-free command is equivalent only when the path contains no whitespace or quotes.
/// With spaces, an unquoted Run command can start a different executable, so do not claim it.
pub fn is_own(current: &str, expected: &str) -> bool {
    let current = current.trim();
    if current.eq_ignore_ascii_case(expected) { return true; }
    if let Some(path) = expected.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        if !path.is_empty() && !path.chars().any(|c| c.is_whitespace() || c == '"') {
            return current.eq_ignore_ascii_case(path);
        }
    }
    false
}

pub fn decide(current: Option<&str>, expected: &str, enabled: bool) -> Change {
    if expected.is_empty() || current.is_some_and(|s| !is_own(s, expected)) {
        return Change::Conflict;
    }
    match (current.is_some(), enabled) {
        (false, true) => Change::Write,
        (true, false) => Change::Delete,
        _ => Change::None,
    }
}

/// The confirmation must authorize this exact value and this exact replacement; no blind
/// overwrite if another actor changed either while the dialog was open.
pub fn replacement_allowed(approved_old: &str, approved_new: &str, actual_old: Option<&str>, actual_new: &str) -> bool {
    !approved_new.is_empty() && actual_old == Some(approved_old) && actual_new == approved_new
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_own_exact_command_is_changed() {
        let ours = r#""C:\Program Files\AuraUI\AuraDock.exe""#;
        assert_eq!(decide(None, ours, true), Change::Write);
        assert_eq!(decide(Some(ours), ours, false), Change::Delete);
        assert_eq!(decide(Some(ours), ours, true), Change::None);
        assert_eq!(decide(None, ours, false), Change::None);
        for foreign in [r#""C:\Other\AuraDock.exe""#, r#""C:\Program Files\AuraUI\AuraDock.exe" --other"#, r#"C:\Program Files\AuraUI\AuraDock.exe"#] {
            assert_eq!(decide(Some(foreign), ours, true), Change::Conflict);
            assert_eq!(decide(Some(foreign), ours, false), Change::Conflict);
        }
    }
    #[test]
    fn accepts_only_safe_equivalent_unquoted_paths() {
        let ours = r#""C:\AuraUI\AuraDock.exe""#;
        assert!(is_own(r"c:\auraui\auradock.exe", ours));
        assert!(is_own(r#"  "C:\AuraUI\AuraDock.exe"  "#, ours));
        assert!(!is_own(r"C:\AuraUI\AuraDock.exe --settings", ours));
        assert!(!is_own(r"C:\AuraUI\AuraDock.exe.bak", ours));
    }
    #[test]
    fn confirmation_applies_only_to_exact_values_seen_by_user() {
        assert!(replacement_allowed("old", "new", Some("old"), "new"));
        assert!(!replacement_allowed("old", "new", Some("changed"), "new"));
        assert!(!replacement_allowed("old", "new", None, "new"));
        assert!(!replacement_allowed("old", "new", Some("old"), "changed"));
        assert!(!replacement_allowed("old", "", Some("old"), ""));
    }
}
