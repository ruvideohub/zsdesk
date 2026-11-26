//! Pure month-grid layout for the independent desktop calendar.

/// The caller supplies Windows' local first weekday (Sunday = 0) and locale's week start.
/// Returned cells are (day, zero-based column, zero-based row).
pub fn cells(days: u32, first_weekday: u32, week_start: u32) -> Vec<(u32, u32, u32)> {
    let offset = (first_weekday % 7 + 7 - week_start % 7) % 7;
    (1..=days.min(31)).map(|day| {
        let cell = offset + day - 1;
        (day, cell % 7, cell / 7)
    }).collect()
}

pub fn month_label(year: u32, month: u32, russian: bool) -> String {
    let ru = ["Январь", "Февраль", "Март", "Апрель", "Май", "Июнь", "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь"];
    let en = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let name = if russian { &ru } else { &en };
    format!("{} {}", name[(month.clamp(1, 12) - 1) as usize], year)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aligns_both_week_starts_and_six_row_months() {
        let monday = cells(31, 0, 1); // Sunday the first, Monday-first display
        assert_eq!(monday[0], (1, 6, 0));
        assert_eq!(monday[30], (31, 1, 5));
        let sunday = cells(31, 0, 0);
        assert_eq!(sunday[0], (1, 0, 0));
        assert_eq!(sunday[30], (31, 2, 4));
    }
    #[test]
    fn no_dates_are_lost_or_repeated() {
        for days in [28, 29, 30, 31] {
            for first in 0..7 {
                for start in 0..2 {
                    let grid = cells(days, first, start);
                    assert_eq!(grid.len(), days as usize);
                    for (i, (day, col, row)) in grid.iter().enumerate() {
                        assert_eq!(*day, i as u32 + 1);
                        assert!(*col < 7 && *row < 6);
                    }
                }
            }
        }
    }
    #[test]
    fn month_names_are_localized_and_bounded() {
        assert_eq!(month_label(2026, 2, true), "Февраль 2026");
        assert_eq!(month_label(2026, 2, false), "February 2026");
        assert_eq!(month_label(2026, 0, false), "January 2026");
        assert_eq!(month_label(2026, 99, false), "December 2026");
    }
}
