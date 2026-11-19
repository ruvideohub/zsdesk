//! Pure dock geometry: the panel rectangle, one slot per icon and the icon
//! boxes produced by the magnification curve.
//!
//! Everything here is in physical pixels and free of Win32 types, so the
//! layout can be exercised by unit tests. `dock.rs` converts the result into
//! Direct2D rectangles when it paints.

use crate::magnify;

/// Plain rectangle; `w`/`h` are never negative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w: w.max(0.0), h: h.max(0.0) }
    }

    pub fn right(&self) -> f32 { self.x + self.w }
    pub fn bottom(&self) -> f32 { self.y + self.h }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// Sizes of one dock at 100 % scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Icon edge length.
    pub icon: f32,
    /// Space between two icons.
    pub gap: f32,
    /// Inset between the panel edge and the first icon.
    pub padding: f32,
    /// Visible panel height. Magnified icons may rise above it.
    pub panel: f32,
}

impl Metrics {
    /// Scales every length; used for DPI awareness.
    pub fn at_scale(self, factor: f32) -> Self {
        let factor = factor.clamp(0.5, 4.0);
        Self {
            icon: self.icon * factor,
            gap: self.gap * factor,
            padding: self.padding * factor,
            panel: self.panel * factor,
        }
    }

    /// Distance between two icon centers without magnification.
    pub fn pitch(&self) -> f32 { self.icon + self.gap }

    /// Height a window needs above the panel to show magnified icons.
    pub fn headroom(&self, max_scale: f32) -> f32 {
        (self.icon * (max_scale.clamp(1.0, 3.0) - 1.0)).max(0.0)
    }
}

/// Magnification settings as the layout sees them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Magnify {
    /// Largest scale, applied under the pointer.
    pub max: f32,
    /// Influence radius measured in icon pitches.
    pub influence: f32,
}

/// One icon: the slot it occupies and the (possibly magnified) icon inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemBox {
    /// Slot rectangle inside the panel; clicks are hit-tested against it.
    pub slot: Rect,
    /// Icon rectangle, bottom-aligned inside the slot.
    pub icon: Rect,
    /// Scale applied to this icon (1.0 without magnification).
    pub scale: f32,
}

/// Where the panel and its icons go inside the dock window.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub panel: Rect,
    pub items: Vec<ItemBox>,
}

impl Placement {
    /// Item index under a window-space point, if any.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.items.iter().position(|item| item.slot.contains(x, y))
    }

}

/// Vertical offset of the panel while the dock slides off screen: `0` when
/// fully shown, `panel - sliver` when only the strip at the bottom edge is
/// visible. The panel is transformed, never resized, so the icons keep their
/// relative positions during the animation.
pub fn hidden_offset(reveal: f32, panel: f32, sliver: f32) -> f32 {
    let hidden = (panel - sliver.max(0.0)).max(0.0);
    hidden * (1.0 - reveal.clamp(0.0, 1.0))
}

/// Lays `count` icons out inside `area`, centered horizontally with the panel
/// resting on the bottom edge of `area`.
///
/// `pointer_x` is the pointer position in window space and only takes effect
/// when `magnify` is `Some`; the panel then widens around its center exactly
/// like the macOS dock does.
///
/// `reserve` is space the caller keeps on the right edge for the status sector
/// (see [`crate::status`]); the panel grows by that much and stays centred.
pub fn place(count: usize, area: Rect, metrics: Metrics, pointer_x: Option<f32>, magnify: Option<Magnify>, reserve: f32) -> Placement {
    let panel_top = area.bottom() - metrics.panel;
    if count == 0 {
        let width = metrics.padding * 2.0 + reserve.max(0.0);
        let panel = Rect::new(area.x + (area.w - width) / 2.0, panel_top, width, metrics.panel);
        return Placement { panel, items: Vec::new() };
    }

    // Base (unmagnified) centers decide which icon the pointer is closest to;
    // a layout that fed magnified centers back into the curve would oscillate.
    let reserve = reserve.max(0.0);
    let base_width = metrics.padding * 2.0 + count as f32 * metrics.icon + (count - 1) as f32 * metrics.gap + reserve;
    let base_left = area.x + (area.w - base_width) / 2.0;

    let mut scales = vec![1.0f32; count];
    if let (Some(pointer), Some(magnification)) = (pointer_x, magnify) {
        if pointer >= area.x && pointer <= area.right() {
            let radius = (magnification.influence.max(0.25) * metrics.pitch()).max(1.0);
            for (index, scale) in scales.iter_mut().enumerate() {
                let center = base_left + metrics.padding + index as f32 * metrics.pitch() + metrics.icon / 2.0;
                *scale = magnify::scale(pointer - center, radius, magnification.max);
            }
        }
    }

    let mut gap = metrics.gap;
    let mut padding = metrics.padding;
    let mut total = padding * 2.0 + gap * (count - 1) as f32 + reserve
        + scales.iter().map(|scale| metrics.icon * scale).sum::<f32>();
    // The dock never pokes out of its window: tighten the gaps first, then the
    // padding, and only then accept a slightly clipped row on tiny screens.
    if total > area.w {
        let available = (area.w - padding * 2.0 - metrics.icon * count as f32 - reserve).max(0.0);
        gap = if count > 1 { (available / (count - 1) as f32).clamp(0.0, metrics.gap) } else { 0.0 };
        total = padding * 2.0 + gap * (count - 1) as f32 + metrics.icon * count as f32 + reserve;
    }
    if total > area.w {
        padding = ((area.w - metrics.icon * count as f32 - gap * (count - 1) as f32 - reserve) / 2.0).max(0.0);
        total = (padding * 2.0 + gap * (count - 1) as f32 + metrics.icon * count as f32 + reserve).min(area.w);
    }

    let left = area.x + (area.w - total) / 2.0;
    let panel = Rect::new(left, panel_top, total, metrics.panel);
    // Icons sit on the panel floor; the padding above them is what grows with
    // the magnification, so the icons always rest on the same baseline.
    let icon_bottom = panel.bottom() - metrics.padding.min((metrics.panel - metrics.icon).max(0.0) / 2.0).max(0.0);

    let mut items = Vec::with_capacity(count);
    let mut cursor = left + padding;
    for scale in scales {
        let size = metrics.icon * scale;
        let slot = Rect::new(cursor, panel.y, size, metrics.panel);
        // Magnified icons grow upwards and stay centered on their slot.
        let icon = Rect::new(cursor, icon_bottom - size, size, size);
        items.push(ItemBox { slot, icon, scale });
        cursor += size + gap;
    }
    Placement { panel, items }
}

#[cfg(test)]
mod tests {
    use super::*;

    const METRICS: Metrics = Metrics { icon: 48.0, gap: 8.0, padding: 10.0, panel: 64.0 };
    const MAGNIFY: Magnify = Magnify { max: 1.8, influence: 2.5 };
    const AREA: Rect = Rect::new(0.0, 0.0, 1200.0, 120.0);

    #[test]
    fn without_magnification_the_panel_is_a_plain_row() {
        let placement = place(5, AREA, METRICS, Some(600.0), None, 0.0);
        assert_eq!(placement.panel.w, 2.0 * 10.0 + 5.0 * 48.0 + 4.0 * 8.0);
        assert!(placement.items.iter().all(|item| item.scale == 1.0));
        assert_eq!(placement.panel.bottom(), AREA.bottom());
        // The row is centered.
        assert!((placement.panel.x + placement.panel.w / 2.0 - 600.0).abs() < 0.01);
    }

    #[test]
    fn the_reserve_widens_the_panel_without_moving_the_row() {
        let plain = place(5, AREA, METRICS, None, None, 0.0);
        let reserved = place(5, AREA, METRICS, None, None, 120.0);
        assert!((reserved.panel.w - plain.panel.w - 120.0).abs() < 0.01);
        // Both stay centred and keep the icons where they were.
        assert!((reserved.panel.x + reserved.panel.w / 2.0 - 600.0).abs() < 0.01);
        // The panel is 120 px wider, so it starts 60 px further left.
        assert!((reserved.items[0].slot.x - plain.items[0].slot.x + 60.0).abs() < 0.01);
    }

    #[test]
    fn items_never_overlap_and_keep_their_order() {
        let placement = place(7, AREA, METRICS, Some(610.0), Some(MAGNIFY), 0.0);
        for pair in placement.items.windows(2) {
            assert!(pair[0].slot.right() <= pair[1].slot.x + 0.01);
        }
        assert!(placement.items.windows(2).all(|pair| pair[0].icon.x < pair[1].icon.x));
    }

    #[test]
    fn the_hovered_icon_reaches_the_configured_scale() {
        let base = place(7, AREA, METRICS, None, Some(MAGNIFY), 0.0);
        let target = base.items[3].slot.x + base.items[3].slot.w / 2.0;
        let hovered = place(7, AREA, METRICS, Some(target), Some(MAGNIFY), 0.0);
        assert!((hovered.items[3].scale - MAGNIFY.max).abs() < 1e-3);
        assert!(hovered.items[2].scale > 1.0 && hovered.items[2].scale < MAGNIFY.max);
        assert!(hovered.panel.w > base.panel.w);
        // Neighbours on both sides grow, distant icons do not.
        assert!(hovered.items[4].scale > 1.0);
        assert_eq!(hovered.items[0].scale, 1.0);
    }

    #[test]
    fn the_pointer_outside_the_window_does_not_magnify() {
        let placement = place(4, AREA, METRICS, Some(-40.0), Some(MAGNIFY), 0.0);
        assert!(placement.items.iter().all(|item| item.scale == 1.0));
        assert_eq!(place(4, AREA, METRICS, None, Some(MAGNIFY), 0.0).items[0].scale, 1.0);
    }

    #[test]
    fn a_narrow_window_tightens_gaps_and_stays_inside() {
        let narrow = Rect::new(0.0, 0.0, 320.0, 120.0);
        let placement = place(8, narrow, METRICS, Some(160.0), Some(MAGNIFY), 0.0);
        assert!(placement.panel.w <= narrow.w + 0.01);
        assert!(placement.panel.x >= narrow.x - 0.01);
        assert!(placement.panel.right() <= narrow.right() + 0.01);
    }

    #[test]
    fn an_empty_dock_still_has_a_panel() {
        let placement = place(0, AREA, METRICS, Some(10.0), Some(MAGNIFY), 0.0);
        assert!(placement.items.is_empty());
        assert_eq!(placement.panel.w, METRICS.padding * 2.0);
        assert_eq!(placement.panel.h, METRICS.panel);
    }

    #[test]
    fn the_hidden_dock_leaves_only_the_sliver_on_screen() {
        assert_eq!(hidden_offset(1.0, 64.0, 4.0), 0.0);
        assert_eq!(hidden_offset(0.0, 64.0, 4.0), 60.0);
        assert_eq!(hidden_offset(0.5, 64.0, 4.0), 30.0);
        // A reveal value outside 0..=1 cannot push the panel off the window.
        assert_eq!(hidden_offset(-3.0, 64.0, 4.0), 60.0);
        assert_eq!(hidden_offset(4.0, 64.0, 4.0), 0.0);
    }

    #[test]
    fn hit_testing_matches_the_icon_slots() {
        let placement = place(3, AREA, METRICS, None, None, 0.0);
        let middle = placement.items[1].slot;
        assert_eq!(placement.hit(middle.x + 1.0, middle.y + 1.0), Some(1));
        assert_eq!(placement.hit(1.0, 1.0), None);
    }

    #[test]
    fn metrics_scale_with_dpi_and_reserve_headroom() {
        let scaled = METRICS.at_scale(1.5);
        assert_eq!(scaled.icon, 72.0);
        assert_eq!(scaled.panel, 96.0);
        assert!((METRICS.headroom(1.8) - 38.4).abs() < 1e-3);
        assert_eq!(METRICS.headroom(1.0), 0.0);
        assert_eq!(METRICS.at_scale(100.0).icon, METRICS.icon * 4.0);
    }
}
