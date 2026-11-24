//! Pure appbar positioning policy, independent of Win32.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub proposed: [i32; 4],
    pub assigned: [i32; 4],
}

/// Skip redundant ABM_SETPOS calls (which trigger ABN_POSCHANGED). Keep the
/// Shell's actual assigned rectangle, which can differ from our proposal.
pub fn cached_assignment(last: Option<Position>, proposed: [i32; 4]) -> Option<[i32; 4]> {
    last.filter(|p| p.proposed == proposed).map(|p| p.assigned)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_registration_must_be_announced() {
        assert_eq!(cached_assignment(None, [0, 0, 1920, 40]), None);
    }
    #[test]
    fn unchanged_position_uses_actual_assigned_rectangle() {
        let proposed = [-1920, 1000, 0, 1040];
        let assigned = [-1920, 990, 0, 1030];
        assert_eq!(
            cached_assignment(Some(Position { proposed, assigned }), proposed),
            Some(assigned)
        );
    }
    #[test]
    fn changes_to_any_edge_require_a_new_announcement() {
        let proposed = [0, 0, 1920, 40];
        let previous = Some(Position {
            proposed,
            assigned: proposed,
        });
        for i in 0..4 {
            let mut changed = proposed;
            changed[i] += 1;
            assert_eq!(cached_assignment(previous, changed), None);
        }
    }
}
