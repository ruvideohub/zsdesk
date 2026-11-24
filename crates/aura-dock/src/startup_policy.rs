//! Safe ownership rules for Dock's per-user login entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change { None, Write, Delete, Conflict }

pub fn decide(current: Option<&str>, expected: &str, enabled: bool) -> Change {
    if expected.is_empty() || current.is_some_and(|s| !s.trim().eq_ignore_ascii_case(expected)) {
        return Change::Conflict;
    }
    match (current.is_some(), enabled) {
        (false, true) => Change::Write,
        (true, false) => Change::Delete,
        _ => Change::None,
    }
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
        for foreign in [r#""C:\Other\AuraDock.exe""#, r#""C:\Program Files\AuraUI\AuraDock.exe" --other"#] {
            assert_eq!(decide(Some(foreign), ours, true), Change::Conflict);
            assert_eq!(decide(Some(foreign), ours, false), Change::Conflict);
        }
    }
}
