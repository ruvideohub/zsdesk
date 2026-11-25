//! Pure ordering rule: enumeration is front to back; ignore shell and other cards.
/// Index of the lowest eligible window underneath this card, if it needs lowering.
pub fn last_below(card: usize, eligible: &[bool]) -> Option<usize> {
    eligible.iter().enumerate().skip(card.saturating_add(1)).rev()
        .find_map(|(index, &include)| include.then_some(index))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lowers_below_the_lowest_app_but_not_behind_desktop() {
        // App, card, app, app, shell desktop (in z order).
        assert_eq!(last_below(1, &[true, false, true, true, false]), Some(3));
        assert_eq!(last_below(3, &[true, true, true, false, false]), None);
        // Cards and desktop are ineligible; no unnecessary reshuffle.
        assert_eq!(last_below(0, &[false, false, false]), None);
        assert_eq!(last_below(99, &[true, true]), None);
    }
}
