//! Geometry of the status sector on the right edge of the panel.
//!
//! The sector holds the small system indicators — hidden icons, network,
//! volume and battery — and finishes with the clock. It is laid out from the
//! right edge inwards, so adding or removing an indicator never moves the
//! clock and never moves the icons: the panel simply grows to the left.
//!
//! No Win32 types are involved, so the arithmetic is unit-tested on any host.

use crate::layout::Rect;

/// Edge length of one indicator at 100 % scale, in pixels.
pub const ITEM: f32 = 26.0;
/// Space between two indicators.
pub const GAP: f32 = 3.0;
/// Width kept on the right edge for the clock text.
pub const CLOCK: f32 = 96.0;

/// One indicator of the status sector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Opens the system flyouts; the place where hidden icons would live.
    Chevron,
    Network,
    Volume,
    Battery,
}

impl Kind {
    /// Order of the indicators from left to right.
    pub const ALL: [Kind; 4] = [Kind::Chevron, Kind::Network, Kind::Volume, Kind::Battery];

    /// Translation key of the hover label and the menu header.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Chevron => "隐藏的图标",
            Kind::Network => "网络",
            Kind::Volume => "声音",
            Kind::Battery => "电池",
        }
    }
}

/// Space the panel must reserve for `kinds`, behind the clock area.
pub fn reserve(kinds: &[Kind], item: f32, gap: f32) -> f32 {
    if kinds.is_empty() {
        return 0.0;
    }
    let count = kinds.len() as f32;
    gap * (count - 1.0) + item * count
}

/// Indicator rectangles inside `panel`, right-aligned against the clock area.
///
/// Indicators are vertically centred on the panel floor line, which keeps them
/// on the same baseline as the clock when the panel height changes.
pub fn layout(panel: Rect, kinds: &[Kind], item: f32, gap: f32, clock: f32) -> Vec<(Kind, Rect)> {
    let width = reserve(kinds, item, gap);
    let bottom = panel.bottom() - (panel.h - item) / 2.0;
    let mut cursor = panel.right() - clock - width;
    let mut items = Vec::with_capacity(kinds.len());
    for kind in kinds {
        items.push((*kind, Rect::new(cursor, bottom - item, item, item)));
        cursor += item + gap;
    }
    items
}

/// Indicator under a window-space point.
pub fn hit(items: &[(Kind, Rect)], x: f32, y: f32) -> Option<Kind> {
    items.iter().find(|(_, rect)| rect.contains(x, y)).map(|(kind, _)| *kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: Rect = Rect::new(100.0, 40.0, 600.0, 64.0);

    fn items() -> Vec<(Kind, Rect)> {
        layout(PANEL, &Kind::ALL, ITEM, GAP, CLOCK)
    }

    #[test]
    fn the_reserve_matches_the_drawn_items() {
        let width = reserve(&Kind::ALL, ITEM, GAP);
        assert_eq!(width, 4.0 * ITEM + 3.0 * GAP);
        assert_eq!(reserve(&[], ITEM, GAP), 0.0);
    }

    #[test]
    fn indicators_are_right_aligned_before_the_clock() {
        let items = items();
        let last = items.last().unwrap().1;
        assert!((last.right() - (PANEL.right() - CLOCK)).abs() < 0.01);
        let first = items.first().unwrap().1;
        assert!((first.x - (PANEL.right() - CLOCK - reserve(&Kind::ALL, ITEM, GAP))).abs() < 0.01);
    }

    #[test]
    fn indicators_never_overlap_and_keep_their_order() {
        let items = items();
        assert_eq!(items.len(), Kind::ALL.len());
        for (index, pair) in items.windows(2).enumerate() {
            assert_eq!(pair[0].0, Kind::ALL[index]);
            assert!(pair[0].1.right() <= pair[1].1.x + 0.01);
        }
    }

    #[test]
    fn the_sector_stays_inside_the_panel() {
        for (_, rect) in items() {
            assert!(rect.x >= PANEL.x);
            assert!(rect.bottom() <= PANEL.bottom() + 0.01);
            assert!(rect.y >= PANEL.y);
        }
    }

    #[test]
    fn clicks_resolve_to_the_indicator_under_the_pointer() {
        let items = items();
        let (kind, rect) = items[2];
        assert_eq!(hit(&items, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0), Some(kind));
        assert_eq!(hit(&items, PANEL.x + 4.0, PANEL.y + 4.0), None);
        // The clock area belongs to the clock, not to the sector.
        assert_eq!(hit(&items, PANEL.right() - 20.0, rect.y + 2.0), None);
    }

    #[test]
    fn a_hidden_sector_reserves_nothing() {
        assert_eq!(reserve(&[], ITEM, GAP), 0.0);
        assert!(layout(PANEL, &[], ITEM, GAP, CLOCK).is_empty());
    }
}
