//! Geometry and hit testing of the "sound and status" card above the dock.
//!
//! The card is drawn by [`crate::panel_window`]; everything that can be decided
//! without a window lives here, so the layout, the slider mapping and the click
//! targets are unit-tested on any host.

use crate::layout::Rect;

/// Card width at 100 % scale, in DIPs.
pub const WIDTH: f32 = 322.0;

const PAD: f32 = 16.0;
const HEADER: f32 = 18.0;
const GAP: f32 = 9.0;
const SLIDER: f32 = 26.0;
const MUTE: f32 = 26.0;
const MUTE_GAP: f32 = 10.0;
const SEP_TOP: f32 = 12.0;
const SEP_BOTTOM: f32 = 11.0;
const ROW: f32 = 22.0;
const CHIP: f32 = 34.0;
const CHIP_GAP: f32 = 7.0;
const BOTTOM: f32 = 15.0;

/// Everything the card shows, in card-local coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// Scale the layout was built for (DPI factor).
    pub scale: f32,
    /// The whole card starting at the origin.
    pub card: Rect,
    /// "Звук" caption.
    pub header: Rect,
    /// Percentage on the right of the caption.
    pub value: Rect,
    /// Mute button on the left of the slider.
    pub mute: Rect,
    /// Slider rail, the draggable part.
    pub track: Rect,
    /// Network row, then battery row.
    pub rows: [Rect; 2],
    /// Four action chips: quick settings, notifications, sound, network.
    pub chips: [Rect; 4],
}

/// Part of the card under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Mute,
    Track,
    Row(usize),
    Chip(usize),
}

impl Layout {
    /// Card-local point to the element under it.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if !self.card.contains(x, y) {
            return None;
        }
        if self.mute.contains(x, y) {
            return Some(Hit::Mute);
        }
        if self.track.contains(x, y) {
            return Some(Hit::Track);
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.contains(x, y) {
                return Some(Hit::Row(index));
            }
        }
        for (index, chip) in self.chips.iter().enumerate() {
            if chip.contains(x, y) {
                return Some(Hit::Chip(index));
            }
        }
        None
    }

    /// Volume the slider would report for a pointer at `x`.
    pub fn volume_at(&self, x: f32) -> u8 {
        if self.track.w <= 0.0 {
            return 0;
        }
        let ratio = ((x - self.track.x) / self.track.w).clamp(0.0, 1.0);
        (ratio * 100.0).round() as u8
    }

    /// Horizontal centre of the slider knob for a volume.
    pub fn knob_x(&self, volume: u8) -> f32 {
        self.track.x + self.track.w * (volume.min(100) as f32 / 100.0)
    }
}

/// Builds the card layout for a DPI factor.
pub fn layout(factor: f32) -> Layout {
    let scale = if factor.is_finite() { factor.clamp(0.75, 4.0) } else { 1.0 };
    let f = |dip: f32| dip * scale;
    let inner = f(WIDTH - PAD * 2.0);
    let height = f(PAD + HEADER + GAP + SLIDER + SEP_TOP + 1.0 + SEP_BOTTOM
        + ROW * 2.0 + GAP + CHIP * 2.0 + CHIP_GAP + BOTTOM);
    let card = Rect::new(0.0, 0.0, f(WIDTH), height);
    let left = f(PAD);

    let mut y = f(PAD);
    let header = Rect::new(left, y, inner, f(HEADER));
    let value = Rect::new(left, y, inner, f(HEADER));
    y += f(HEADER + GAP);

    let mute = Rect::new(left, y, f(MUTE), f(SLIDER));
    let track = Rect::new(left + f(MUTE + MUTE_GAP), y, inner - f(MUTE + MUTE_GAP), f(SLIDER));
    y += f(SLIDER + SEP_TOP + 1.0 + SEP_BOTTOM);

    let rows = [Rect::new(left, y, inner, f(ROW)), Rect::new(left, y + f(ROW), inner, f(ROW))];
    y += f(ROW * 2.0 + GAP);

    let chip_width = (inner - f(CHIP_GAP)) / 2.0;
    let mut chips = [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
    for index in 0..4 {
        let column = index % 2;
        let row = index / 2;
        chips[index] = Rect::new(left + column as f32 * (chip_width + f(CHIP_GAP)),
            y + row as f32 * f(CHIP + CHIP_GAP), chip_width, f(CHIP));
    }
    Layout { scale, card, header, value, mute, track, rows, chips }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_grows_with_the_scale() {
        let plain = layout(1.0);
        let dense = layout(1.5);
        assert!((dense.card.w - plain.card.w * 1.5).abs() < 0.01);
        assert!((dense.card.h - plain.card.h * 1.5).abs() < 0.01);
        assert!((plain.card.w - WIDTH).abs() < 0.01);
    }

    #[test]
    fn a_broken_scale_is_clamped() {
        assert_eq!(layout(f32::NAN).scale, 1.0);
        assert_eq!(layout(0.1).scale, 0.75);
        assert_eq!(layout(9.0).scale, 4.0);
    }

    #[test]
    fn every_element_sits_inside_the_card_without_overlapping() {
        for factor in [0.75, 1.0, 1.25, 1.5, 2.0] {
            let l = layout(factor);
            let inside = |rect: &Rect| {
                rect.x >= l.card.x - 0.01
                    && rect.y >= l.card.y - 0.01
                    && rect.right() <= l.card.right() + 0.01
                    && rect.bottom() <= l.card.bottom() + 0.01
            };
            assert!(inside(&l.header) && inside(&l.value) && inside(&l.mute) && inside(&l.track));
            for row in &l.rows { assert!(inside(row)); }
            for chip in &l.chips { assert!(inside(chip)); }
            // The mute button and the slider share one row and never overlap.
            assert!(l.mute.right() <= l.track.x + 0.01);
            assert!(l.rows[0].bottom() <= l.rows[1].y + 0.01);
            // Chips are two columns, two rows.
            assert!(l.chips[0].right() <= l.chips[1].x + 0.01);
            assert!(l.chips[0].bottom() <= l.chips[2].y + 0.01);
        }
    }

    #[test]
    fn the_slider_maps_to_a_volume() {
        let l = layout(1.0);
        assert_eq!(l.volume_at(l.track.x - 40.0), 0);
        assert_eq!(l.volume_at(l.track.right() + 40.0), 100);
        let middle = l.volume_at(l.track.x + l.track.w / 2.0);
        assert!((49..=51).contains(&middle), "{middle}");
        // The knob lands where the pointer does.
        assert!((l.knob_x(50) - (l.track.x + l.track.w / 2.0)).abs() < 0.5);
        assert_eq!(l.knob_x(200), l.track.right());
    }

    #[test]
    fn clicks_resolve_to_the_right_target() {
        let l = layout(1.0);
        let centre = |rect: &Rect| (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let (x, y) = centre(&l.mute);
        assert_eq!(l.hit(x, y), Some(Hit::Mute));
        let (x, y) = centre(&l.track);
        assert_eq!(l.hit(x, y), Some(Hit::Track));
        let (x, y) = centre(&l.rows[1]);
        assert_eq!(l.hit(x, y), Some(Hit::Row(1)));
        let (x, y) = centre(&l.chips[3]);
        assert_eq!(l.hit(x, y), Some(Hit::Chip(3)));
        // Padding and outside the card are dead zones.
        assert_eq!(l.hit(l.card.right() + 4.0, 4.0), None);
        assert_eq!(l.hit(2.0, 2.0), None);
    }
}
