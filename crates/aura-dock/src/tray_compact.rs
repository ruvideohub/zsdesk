//! Policy for the compact tray in Control Center (no platform dependencies).

pub const PAGE_SIZE: usize = 7;
const SYSTEM_VOLUME: &str = "guid:7820ae73-23e3-4229-82c1-e41cb67d5b9c";

/// Only filter the known legacy system icon, never an app by its translated tooltip.
pub fn show_icon(stable_id: &str, app_hidden: bool) -> bool {
    !app_hidden && !stable_id.eq_ignore_ascii_case(SYSTEM_VOLUME)
}

pub fn page_count(len: usize) -> usize {
    len.saturating_add(PAGE_SIZE - 1) / PAGE_SIZE
}

pub fn clamp_page(page: usize, len: usize) -> usize {
    page.min(page_count(len).saturating_sub(1))
}

pub fn page_range(page: usize, len: usize) -> std::ops::Range<usize> {
    let start = clamp_page(page, len) * PAGE_SIZE;
    start..start.saturating_add(PAGE_SIZE).min(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_icon_is_reachable_and_page_is_clamped() {
        for len in 0..1000 {
            let all: Vec<usize> = (0..page_count(len))
                .flat_map(|p| page_range(p, len))
                .collect();
            assert_eq!(all, (0..len).collect::<Vec<_>>());
            assert_eq!(page_range(usize::MAX, len).end, len);
        }
    }
    #[test]
    fn only_known_system_volume_and_app_hidden_are_excluded() {
        assert!(!show_icon(
            "GUID:7820AE73-23E3-4229-82C1-E41CB67D5B9C",
            false
        ));
        assert!(!show_icon("other", true));
        assert!(show_icon("audio-app.exe#1", false));
        assert!(show_icon(
            "guid:7820ae73-23e3-4229-82c1-e41cb67d5b9d",
            false
        ));
    }
}
