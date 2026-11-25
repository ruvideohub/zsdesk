//! Pure appbar positioning policy, independent of Win32.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub proposed: [i32; 4],
    pub assigned: [i32; 4],
}

/// Reserve exactly from the monitor edge to the inner edge of the visible shelf.
/// Its outer margin is part of that distance; an additional inner margin would
/// leave an empty strip between maximized windows and the panel.
pub fn shelf_reservation(shelf_height: f32, outer_margin: f32) -> i32 {
    (shelf_height.max(0.0) + outer_margin.max(0.0)).ceil() as i32
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
    fn reservation_reaches_shelf_inner_edge_without_a_second_margin() {
        for (shelf, margin, expected) in [(70.0, 8.0, 78), (87.5, 10.0, 98), (105.0, 12.0, 117), (0.0, 0.0, 0)] {
            assert_eq!(shelf_reservation(shelf, margin), expected);
            for edge in [0, 1080, -1440] {
                let strip = shelf_reservation(shelf, margin) as f32;
                let visible_inner_top = edge as f32 + shelf + margin;
                let visible_inner_bottom = edge as f32 - shelf - margin;
                assert!((edge as f32 + strip - visible_inner_top).abs() < 1.0);
                assert!((edge as f32 - strip - visible_inner_bottom).abs() < 1.0);
            }
        }
    }
    #[test]
    fn reservation_does_not_include_magnification_or_label_headroom() {
        assert_eq!(shelf_reservation(70.0, 8.0), 78);
        assert_eq!(shelf_reservation(-1.0, -1.0), 0);
    }
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
