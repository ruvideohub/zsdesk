//! One look for every settings page in AuraUI.
//!
//! The numbers and colours here are the ones the dock's own settings have used since 0.3.20 —
//! a 26 px margin, 56 px rows on one rounded card, a segmented control for sections, switches
//! that are 40 × 24, an accent that is the same blue everywhere. A component draws its page
//! with these functions and therefore looks like the rest of the suite without sharing a single
//! line of component code: this module knows nothing about components, only about a page.
//!
//! Layout is separated from painting on purpose — `tab_rects`, `row_rect` and `knob` are pure
//! arithmetic that can be tested without a window, and `draw_*` only paints what they return.

use crate::gfx::{Align, Canvas, Color, RectF, TextStyle};

/// The margin every page starts from.
pub const PAD: f32 = 26.0;
/// One setting takes this much vertical room.
pub const ROW: f32 = 56.0;
/// A card's corner radius.
pub const CARD: f32 = 14.0;
/// A switch: width, height.
pub const TOGGLE: (f32, f32) = (40.0, 24.0);
/// A section tab.
pub const TAB: (f32, f32) = (104.0, 34.0);
/// The height of the title line.
pub const TITLE: f32 = 32.0;

/// Colours and the one scale factor for a page.
///
/// Dark is the normal AuraUI look; light exists because the suite follows the system theme and
/// a white page in a dark window (or the other way round) looks broken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub dark: bool,
    pub scale: f32,
    /// The window's own background.
    pub background: Color,
    /// Text that says what things are.
    pub text: Color,
    /// Text that explains them.
    pub text2: Color,
    /// Text that is only a hint.
    pub text3: Color,
    /// Separators inside a card.
    pub hair: Color,
    /// A card behind a group of rows.
    pub card: Color,
    /// The tray behind a segmented control.
    pub tray: Color,
    /// The selected segment.
    pub pill: Color,
    /// An unused track (a switch that is off, a slider rail).
    pub track: Color,
    /// The accent: selected, on, focused.
    pub accent: Color,
    /// Text on top of the accent.
    pub on_accent: Color,
    /// A thing that works.
    pub ok: Color,
    /// A thing that is on its way.
    pub warn: Color,
    /// A thing that failed.
    pub bad: Color,
}

impl Look {
    /// The suite's one palette. `scale` is the window's DPI scale (1.0 at 96 dpi).
    pub fn of(dark: bool, scale: f32) -> Self {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        if dark {
            Self {
                dark,
                scale,
                background: Color::rgba8(0x16, 0x1b, 0x21, 255),
                text: Color::rgba8(0xe9, 0xed, 0xf2, 255),
                text2: Color::rgba8(0xa6, 0xb0, 0xbc, 255),
                text3: Color::rgba8(0x74, 0x7f, 0x8c, 255),
                hair: Color::rgba8(0xff, 0xff, 0xff, 20),
                card: Color::rgba8(0xff, 0xff, 0xff, 16),
                tray: Color::rgba8(0xff, 0xff, 0xff, 26),
                pill: Color::rgba8(0xff, 0xff, 0xff, 56),
                track: Color::rgba8(0xff, 0xff, 0xff, 51),
                accent: Color::rgba8(0x4a, 0x9e, 0xff, 255),
                on_accent: Color::rgba8(0xff, 0xff, 0xff, 255),
                ok: Color::rgba8(0x5c, 0xc0, 0x7a, 255),
                warn: Color::rgba8(0xe8, 0xa8, 0x3c, 255),
                bad: Color::rgba8(0xe0, 0x6c, 0x6c, 255),
            }
        } else {
            Self {
                dark,
                scale,
                background: Color::rgba8(0xf6, 0xf7, 0xf9, 255),
                text: Color::rgba8(0x1b, 0x20, 0x26, 255),
                text2: Color::rgba8(0x55, 0x60, 0x6c, 255),
                text3: Color::rgba8(0x7a, 0x85, 0x92, 255),
                hair: Color::rgba8(0x00, 0x00, 0x00, 18),
                card: Color::rgba8(0xff, 0xff, 0xff, 235),
                tray: Color::rgba8(0x00, 0x00, 0x00, 12),
                pill: Color::rgba8(0x00, 0x00, 0x00, 26),
                track: Color::rgba8(0x00, 0x00, 0x00, 40),
                accent: Color::rgba8(0x0a, 0x7f, 0xb8, 255),
                on_accent: Color::rgba8(0xff, 0xff, 0xff, 255),
                ok: Color::rgba8(0x1f, 0x8a, 0x4c, 255),
                warn: Color::rgba8(0xa8, 0x6a, 0x00, 255),
                bad: Color::rgba8(0xb0, 0x2c, 0x2c, 255),
            }
        }
    }

    /// A length in device pixels.
    pub fn px(&self, value: f32) -> f32 {
        value * self.scale
    }

    /// A font size in device pixels.
    pub fn font(&self, size: f32) -> f32 {
        size * self.scale
    }

    /// The card that holds one section's rows, for `count` rows placed from `top`.
    pub fn section_card(&self, page: RectF, top: f32, count: usize) -> RectF {
        RectF::new(page.x + self.px(PAD), top, page.w - self.px(PAD) * 2.0, self.px(ROW) * count as f32)
    }
}

/// What a row holds on its right: a switch, a button, both, two buttons, or nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    None,
    Toggle,
    /// One button at the right edge.
    Button,
    /// Two buttons side by side, the second at the very right edge.
    TwoButtons,
    /// A settings button, then the switch, with the switch at the right edge.
    ToggleAndButton,
}

/// How wide the right-hand side takes.
pub fn side_width(look: &Look, side: Side) -> f32 {
    match side {
        Side::None => 0.0,
        Side::Toggle => look.px(TOGGLE.0) + look.px(24.0),
        Side::Button => look.px(120.0) + look.px(24.0),
        Side::TwoButtons => look.px(120.0) * 2.0 + look.px(32.0),
        Side::ToggleAndButton => look.px(120.0) + look.px(8.0) + look.px(TOGGLE.0) + look.px(24.0),
    }
}

/// Where each row of a section sits, inside the card returned by `section_card`.
pub fn row_rect(look: &Look, card: RectF, index: usize) -> RectF {
    RectF::new(card.x, card.y + look.px(ROW) * index as f32, card.w, look.px(ROW))
}

/// The line between two rows — inset, the way the dock draws it.
pub fn row_separator(look: &Look, card: RectF, index: usize) -> Option<RectF> {
    if index == 0 {
        return None;
    }
    let y = card.y + look.px(ROW) * index as f32;
    Some(RectF::new(card.x + look.px(14.0), y, card.w - look.px(28.0), 1.0))
}

/// The text column of a row, stopping short of whatever the right-hand side uses.
pub fn row_text(look: &Look, row: RectF, side: Side) -> RectF {
    let width = (row.w - look.px(32.0) - side_width(look, side)).max(look.px(60.0));
    RectF::new(row.x + look.px(16.0), row.y, width, row.h)
}

/// Where the switch of a row sits: vertically centred, `24` px from the right edge.
pub fn toggle_rect(look: &Look, row: RectF) -> RectF {
    let (w, h) = (look.px(TOGGLE.0), look.px(TOGGLE.1));
    RectF::new(row.right() - look.px(24.0) - w, row.y + (row.h - h) / 2.0, w, h)
}

/// Where a row's button sits.
pub fn button_rect(look: &Look, row: RectF, which: usize, count: usize) -> RectF {
    let w = look.px(120.0);
    let h = look.px(28.0);
    let gap = look.px(8.0);
    let right = row.right() - look.px(24.0);
    let index = if count >= 2 { which.min(1) } else { 0 };
    let x = right - w - index as f32 * (w + gap);
    RectF::new(x, row.y + (row.h - h) / 2.0, w, h)
}

/// Where the settings button of a row that also carries a switch sits: left of the switch.
pub fn toggle_side_button(look: &Look, row: RectF) -> RectF {
    let field = toggle_rect(look, row);
    let w = look.px(120.0);
    let h = look.px(28.0);
    RectF::new(field.x - look.px(8.0) - w, row.y + (row.h - h) / 2.0, w, h)
}

/// The knob of a switch, `on` or off.
pub fn knob(look: &Look, field: RectF, on: bool) -> RectF {
    let size = field.h - look.px(4.0);
    let x = if on { field.right() - size - look.px(2.0) } else { field.x + look.px(2.0) };
    RectF::new(x, field.y + look.px(2.0), size, size)
}

/// The tabs of a segmented control, starting at `origin`.
pub fn tab_rects(look: &Look, origin_x: f32, origin_y: f32, count: usize) -> Vec<RectF> {
    let mut rects = Vec::with_capacity(count);
    for index in 0..count {
        rects.push(RectF::new(
            origin_x + look.px(3.0) + index as f32 * look.px(TAB.0),
            origin_y + look.px(3.0),
            look.px(TAB.0),
            look.px(TAB.1) - look.px(6.0),
        ));
    }
    rects
}

/// The tray a segmented control sits in.
pub fn tray_rect(look: &Look, origin_x: f32, origin_y: f32, count: usize) -> RectF {
    RectF::new(
        origin_x,
        origin_y,
        look.px(TAB.0) * count as f32 + look.px(6.0),
        look.px(TAB.1),
    )
}

// ---- painting ---------------------------------------------------------------------

/// Fills the page.
pub fn background(cv: &Canvas, look: &Look, page: RectF) {
    cv.fill(page, look.background);
}

/// The title on the left, the «Готово» link on the right.
pub fn header(cv: &Canvas, look: &Look, page: RectF, title: &str, done: &str, done_hovered: bool) {
    let line = RectF::new(page.x + look.px(PAD), page.y + look.px(22.0), page.w - look.px(PAD) * 2.0, look.px(TITLE));
    cv.text(title, line, &TextStyle::ui(look.font(22.0)).weight(600), look.text);
    if done.is_empty() {
        return;
    }
    let button = done_rect(look, page, done);
    if done_hovered {
        cv.round(button.inset(0.0, look.px(2.0)), look.px(8.0), look.card);
    }
    cv.text(done, button, &TextStyle::ui(look.font(13.0)).weight(600).align(Align::Center), look.accent);
}

/// Where the «Готово» link sits, given the page.
pub fn done_rect(look: &Look, page: RectF, done: &str) -> RectF {
    let width = look.px(24.0) + done.chars().count() as f32 * look.font(13.0) * 0.62;
    RectF::new(page.right() - look.px(PAD) - width, page.y + look.px(20.0), width, look.px(30.0))
}

/// A segmented control. Returns nothing: the caller already knows the rectangles.
pub fn tabs(cv: &Canvas, look: &Look, tray: RectF, tabs: &[RectF], names: &[&str], current: usize) {
    cv.round(tray, look.px(10.0), look.tray);
    for (index, rect) in tabs.iter().enumerate() {
        let Some(name) = names.get(index) else { continue };
        let on = index == current;
        if on {
            cv.round(*rect, look.px(8.0), look.pill);
        }
        let style = TextStyle::ui(look.font(12.5)).weight(if on { 600 } else { 400 }).align(Align::Center);
        cv.text(name, *rect, &style, if on { look.text } else { look.text2 });
    }
}

/// The card behind a section.
pub fn card(cv: &Canvas, look: &Look, card: RectF) {
    cv.round(card, look.px(CARD), look.card);
}

/// A row's label and, under it, what it means.
pub fn row_text_painted(cv: &Canvas, look: &Look, area: RectF, label: &str, hint: &str, tone: Color) {
    if hint.is_empty() {
        cv.text(label, area, &TextStyle::ui(look.font(13.0)), look.text);
        return;
    }
    let label_line = RectF::new(area.x, area.y + look.px(9.0), area.w, look.px(20.0));
    let hint_line = RectF::new(area.x, area.y + look.px(28.0), area.w, look.px(18.0));
    cv.text(label, label_line, &TextStyle::ui(look.font(13.0)), look.text);
    let hint_style = TextStyle::ui(look.font(11.0));
    cv.text(hint, hint_line, &hint_style, if tone == look.bad { tone } else { look.text2 });
}

/// A switch.
pub fn toggle(cv: &Canvas, look: &Look, field: RectF, on: bool, hovered: bool, enabled: bool) {
    let fill = if !enabled {
        look.track
    } else if on {
        look.accent
    } else if hovered {
        look.pill
    } else {
        look.track
    };
    cv.round(field, field.h / 2.0, fill);
    cv.round(knob(look, field, on), (field.h - look.px(4.0)) / 2.0, if enabled { look.on_accent } else { look.text3 });
}

/// A button inside a row.
pub fn button(cv: &Canvas, look: &Look, field: RectF, label: &str, hovered: bool, enabled: bool) {
    let fill = if !enabled {
        look.tray
    } else if hovered {
        look.pill
    } else {
        look.card
    };
    cv.round(field, look.px(8.0), fill);
    cv.stroke_round(field, look.px(8.0), if enabled { look.track } else { look.hair }, look.px(1.0));
    let color = if enabled { look.text } else { look.text3 };
    let style = TextStyle::ui(look.font(12.5)).weight(500).align(Align::Center);
    cv.text(label, field, &style, color);
}

/// The accent button — «Готово» at the bottom of a page, «Включить службу».
pub fn accent_button(cv: &Canvas, look: &Look, field: RectF, label: &str, hovered: bool, enabled: bool) {
    let fill = if !enabled { look.track } else if hovered { look.pill } else { look.accent };
    cv.round(field, look.px(8.0), fill);
    let style = TextStyle::ui(look.font(12.5)).weight(600).align(Align::Center);
    cv.text(label, field, &style, if enabled { look.on_accent } else { look.text3 });
}

/// A hairline.
pub fn separator(cv: &Canvas, line: RectF, color: Color) {
    cv.fill(line, color);
}

/// The line at the bottom that says what just happened.
pub fn footer(cv: &Canvas, look: &Look, page: RectF, text: &str, tone: Color) {
    let line = RectF::new(page.x + look.px(PAD), page.bottom() - look.px(34.0), page.w - look.px(PAD) * 2.0, look.px(20.0));
    cv.text(text, line, &TextStyle::ui(look.font(11.5)), tone);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look::of(true, 1.0)
    }

    #[test]
    fn a_row_is_one_uniform_strip_of_the_card() {
        let look = look();
        let card = look.section_card(RectF::new(0.0, 100.0, 600.0, 500.0), 100.0, 3);
        assert_eq!(card.x, PAD);
        assert_eq!(card.w, 600.0 - PAD * 2.0);
        assert_eq!(card.h, ROW * 3.0);
        let row = row_rect(&look, card, 2);
        assert_eq!(row.y, 100.0 + ROW * 2.0);
        assert_eq!(row.h, ROW);
    }

    #[test]
    fn the_first_row_has_no_separator_above_it() {
        let look = look();
        let card = look.section_card(RectF::new(0.0, 0.0, 400.0, 400.0), 0.0, 2);
        assert!(row_separator(&look, card, 0).is_none());
        let line = row_separator(&look, card, 1).unwrap();
        assert_eq!(line.y, ROW);
        assert_eq!(line.w, card.w - 28.0);
    }

    #[test]
    fn a_switch_sits_against_the_right_edge_and_its_knob_moves() {
        let look = look();
        let row = RectF::new(0.0, 0.0, 500.0, ROW);
        let field = toggle_rect(&look, row);
        assert_eq!(field.right(), row.right() - 24.0);
        assert_eq!(field.w, TOGGLE.0);
        assert_eq!(field.y, (ROW - TOGGLE.1) / 2.0);
        let off = knob(&look, field, false);
        let on = knob(&look, field, true);
        assert_eq!(off.x, field.x + 2.0);
        assert_eq!(on.right(), field.right() - 2.0);
        assert!(on.x > off.x);
    }

    #[test]
    fn two_buttons_do_not_overlap() {
        let look = look();
        let row = RectF::new(0.0, 0.0, 500.0, ROW);
        let first = button_rect(&look, row, 0, 2);
        let second = button_rect(&look, row, 1, 2);
        assert_eq!(second.right(), row.right() - 24.0);
        assert!(first.right() <= second.x);
    }

    #[test]
    fn a_settings_button_stands_clear_of_the_switch() {
        let look = look();
        let row = RectF::new(0.0, 0.0, 500.0, ROW);
        let field = toggle_rect(&look, row);
        let button = toggle_side_button(&look, row);
        assert_eq!(field.right(), row.right() - 24.0);
        assert!(button.right() < field.x);
        assert_eq!(button.y + button.h / 2.0, row.y + row.h / 2.0);
        // The text column must stop before the button, or the two would overlap.
        let text = row_text(&look, row, Side::ToggleAndButton);
        assert!(text.right() <= button.x);
    }

    #[test]
    fn tabs_line_up_inside_their_tray() {
        let look = look();
        let tray = tray_rect(&look, 26.0, 60.0, 3);
        let tabs = tab_rects(&look, 26.0, 60.0, 3);
        assert_eq!(tabs.len(), 3);
        assert_eq!(tray.w, TAB.0 * 3.0 + 6.0);
        assert!(tabs[0].x > tray.x);
        assert!(tabs[2].right() <= tray.right());
        assert_eq!(tabs[1].x - tabs[0].x, TAB.0);
    }

    #[test]
    fn the_scale_multiplies_every_measurement() {
        let plain = Look::of(true, 1.0);
        let dense = Look::of(true, 2.0);
        let row = RectF::new(0.0, 0.0, 500.0, plain.px(ROW));
        let small = toggle_rect(&plain, row);
        let big = toggle_rect(&dense, RectF::new(0.0, 0.0, 500.0, dense.px(ROW)));
        assert_eq!(big.w, small.w * 2.0);
    }
}
