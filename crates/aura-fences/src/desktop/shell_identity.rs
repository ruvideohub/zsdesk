//! TaskbarCreated is also sent by replacement taskbars: it is not proof that
//! Explorer or its desktop view actually restarted.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ShellIdentity {
    pub progman: usize,
    pub view: usize,
    pub process: u32,
    pub thread: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Observation { Unavailable, Unchanged, Changed }

pub(super) fn classify(previous: Option<ShellIdentity>, current: Option<ShellIdentity>) -> Observation {
    match current {
        None => Observation::Unavailable,
        Some(identity) if previous == Some(identity) => Observation::Unchanged,
        Some(_) => Observation::Changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> ShellIdentity { ShellIdentity { progman: 1, view: 2, process: 3, thread: 4 } }
    #[test]
    fn a_taskbar_broadcast_is_not_a_shell_restart() {
        let shell = identity();
        assert_eq!(classify(Some(shell), Some(shell)), Observation::Unchanged);
        assert_eq!(classify(Some(shell), None), Observation::Unavailable);
    }
    #[test]
    fn changed_shell_or_new_view_is_reconciled_after_it_becomes_available() {
        let shell = identity();
        assert_eq!(classify(None, Some(shell)), Observation::Changed);
        assert_eq!(classify(Some(shell), Some(ShellIdentity { view: 9, ..shell })), Observation::Changed);
        assert_eq!(classify(Some(shell), Some(ShellIdentity { process: 9, ..shell })), Observation::Changed);
        assert_eq!(classify(Some(shell), Some(ShellIdentity { thread: 9, ..shell })), Observation::Changed);
        assert_eq!(classify(Some(shell), Some(ShellIdentity { progman: 9, ..shell })), Observation::Changed);
    }
}
