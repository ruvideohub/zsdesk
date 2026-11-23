//! Layout, overflow and popup UI for notification-area icons.
//!
//! The protocol host lives in `tray.rs`; this module only arranges the icons the dock shows.

use windows::UI::Composition::{Compositor, CompositionSurfaceBrush, ShapeVisual, SpriteVisual};
use windows::Win32::Foundation::RECT;

use crate::gfx::{Gfx, TextStyle};
use crate::ui::{self, Popup};

pub const MAX_ON_BAR: usize = 5;

/// One icon ready to be drawn by either the bar or the overflow popup.
#[derive(Clone)]
pub struct TrayIconView {
    pub key: u64,
    pub stable_id: String,
    pub tip: String,
    pub brush: Option<CompositionSurfaceBrush>,
    /// The application itself marked this icon hidden; do not display it.
    pub app_hidden: bool,
    pub standard_tip: bool,
}

/// Sort icons using the user's saved order, then put newly registered icons at the end.
pub fn ordered(icons: &[TrayIconView], order: &[String]) -> Vec<TrayIconView> {
    let rank: std::collections::HashMap<&str, usize> = order.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let mut out = icons.to_vec();
    out.sort_by_key(|icon| rank.get(icon.stable_id.as_str()).copied().unwrap_or(usize::MAX));
    out
}

/// Split a complete icon list into the strip and its overflow popup.
pub fn partition(icons: &[TrayIconView], order: &[String], hidden: &[String], max_on_bar: usize) -> (Vec<TrayIconView>, Vec<TrayIconView>) {
    let sorted = ordered(icons, order);
    let mut bar = Vec::new();
    let mut overflow = Vec::new();
    for icon in sorted {
        match crate::tray_policy::placement(icon.app_hidden, hidden.iter().any(|id| id == &icon.stable_id), bar.len() < max_on_bar) {
            crate::tray_policy::Placement::HiddenByApp => {},
            crate::tray_policy::Placement::Strip => bar.push(icon),
            crate::tray_policy::Placement::Overflow => overflow.push(icon),
        }
    }
    (bar, overflow)
}

/// Move one stable identity relative to a target identity. `after` selects the insertion side.
pub fn move_before(order: &mut Vec<String>, moving: &str, target: &str, after: bool) {
    if moving == target {
        return;
    }
    if let Some(i) = order.iter().position(|id| id == moving) {
        order.remove(i);
    }
    if !order.iter().any(|id| id == target) {
        order.push(target.to_string());
    }
    let offset = if after { 1 } else { 0 };
    let at = order.iter().position(|id| id == target).unwrap_or(order.len()) + offset;
    order.insert(at.min(order.len()), moving.to_string());
}

#[derive(Clone)]
struct Cell {
    key: u64,
    stable_id: String,
    tip: String,
    rect: RECT,
    icon_rect: RECT,
    background: ShapeVisual,
}

/// Non-activating popup that holds tray icons hidden from the bar or beyond its visible limit.
pub struct TrayPopup {
    pub popup: Popup,
    pub open: bool,
    pub hover: Option<u64>,
    cells: Vec<Cell>,
    bounds: RECT,
    scale: f32,
    header: Option<SpriteVisual>,
    header_home: String,
    header_x: f32,
    header_y: f32,
    header_max_w: f32,
}

impl TrayPopup {
    pub fn new(c: &Compositor) -> windows::core::Result<Self> {
        Ok(Self {
            popup: Popup::new(c, false)?,
            open: false,
            hover: None,
            cells: Vec::new(),
            bounds: RECT::default(),
            scale: 1.0,
            header: None,
            header_home: "Background apps".into(),
            header_x: 0.0,
            header_y: 0.0,
            header_max_w: 0.0,
        })
    }

    pub fn hwnd(&self) -> windows::Win32::Foundation::HWND {
        self.popup.hwnd
    }

    pub fn bounds(&self) -> RECT {
        self.bounds
    }

    pub fn cell_at(&self, x: f32, y: f32) -> Option<u64> {
        self.cells.iter().find(|cell| inside(cell.rect, x, y)).map(|cell| cell.key)
    }

    pub fn tip(&self, key: u64) -> Option<&str> {
        self.cells.iter().find(|cell| cell.key == key).map(|cell| cell.tip.as_str())
    }

    pub fn stable_id(&self, key: u64) -> Option<&str> {
        self.cells.iter().find(|cell| cell.key == key).map(|cell| cell.stable_id.as_str())
    }

    pub fn screen_rect(&self, key: u64) -> Option<RECT> {
        let cell = self.cells.iter().find(|cell| cell.key == key)?;
        Some(RECT {
            left: self.bounds.left + cell.icon_rect.left,
            top: self.bounds.top + cell.icon_rect.top,
            right: self.bounds.left + cell.icon_rect.right,
            bottom: self.bounds.top + cell.icon_rect.bottom,
        })
    }

    pub fn set_hover(&mut self, key: Option<u64>) -> bool {
        if self.hover == key {
            return false;
        }
        self.hover = key;
        for cell in &self.cells {
            let _ = cell.background.SetOpacity(if Some(cell.key) == key { 1.0 } else { 0.0 });
        }
        true
    }

    /// Show the overflow popup next to the chevron and keep it within the current monitor.
    pub fn show(
        &mut self,
        c: &Compositor,
        gfx: &Gfx,
        backdrop: &windows::UI::Composition::CompositionBrush,
        icons: &[TrayIconView],
        anchor: RECT,
        monitor: RECT,
        below: bool,
        scale: f32,
        shadow: f32,
    ) -> windows::core::Result<()> {
        if icons.is_empty() {
            self.hide();
            return Ok(());
        }
        self.scale = scale;
        self.header_home = crate::strings::tr(&format!("Background apps ({})", icons.len()));
        self.hover = None;
        self.cells.clear();
        self.popup.root.Children()?.RemoveAll()?;

        let s = scale;
        let cols = icons.len().min(6).max(1);
        let rows = icons.len().div_ceil(cols);
        let pad = 13.0 * s;
        let cell = 38.0 * s;
        let header_h = 28.0 * s;
        let icon_size = 20.0 * s;
        let w = (2.0 * pad + cols as f32 * cell).ceil();
        let h = (2.0 * pad + header_h + rows as f32 * cell).ceil();
        let panel = ui::panel(c, gfx, backdrop, w, h, 14.0 * s, ui::theme::surface(0.82), s, shadow)?;
        let outer_w = w + 2.0 * panel.margin;
        let outer_h = h + 2.0 * panel.margin;
        self.popup.root.SetSize(ui::v2(outer_w, outer_h))?;

        let (header, _, _) = ui::text(
            c,
            gfx,
            &self.header_home,
            &TextStyle { px: 12.0 * s, weight: 600, color: ui::theme::text_2(), max_w: Some(w - 2.0 * pad), ..Default::default() },
        )?;
        header.SetOffset(ui::v3(pad, pad + 2.0 * s, 0.0))?;
        panel.content.Children()?.InsertAtTop(&header)?;
        self.header = Some(header);
        self.header_x = pad;
        self.header_y = pad + 2.0 * s;
        self.header_max_w = w - 2.0 * pad;

        for (i, icon) in icons.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let x = pad + col as f32 * cell;
            let y = pad + header_h + row as f32 * cell;
            let (bg, _, _) = ui::rounded(c, cell - 3.0 * s, cell - 3.0 * s, 7.0 * s, ui::theme::fill_2())?;
            bg.SetOffset(ui::v3(x + 1.5 * s, y + 1.5 * s, 0.0))?;
            bg.SetOpacity(0.0)?;
            panel.content.Children()?.InsertAtTop(&bg)?;
            if let Some(brush) = &icon.brush {
                let sprite = c.CreateSpriteVisual()?;
                sprite.SetBrush(brush)?;
                sprite.SetSize(ui::v2(icon_size, icon_size))?;
                sprite.SetOffset(ui::v3(x + (cell - icon_size) / 2.0, y + (cell - icon_size) / 2.0, 0.0))?;
                panel.content.Children()?.InsertAtTop(&sprite)?;
            }
            let left = (panel.margin + x) as i32;
            let top = (panel.margin + y) as i32;
            let right = (panel.margin + x + cell) as i32;
            let bottom = (panel.margin + y + cell) as i32;
            let il = (panel.margin + x + (cell - icon_size) / 2.0) as i32;
            let it = (panel.margin + y + (cell - icon_size) / 2.0) as i32;
            self.cells.push(Cell {
                key: icon.key,
                stable_id: icon.stable_id.clone(),
                tip: icon.tip.clone(),
                rect: RECT { left, top, right, bottom },
                icon_rect: RECT { left: il, top: it, right: il + icon_size as i32, bottom: it + icon_size as i32 },
                background: bg,
            });
        }
        self.popup.root.Children()?.InsertAtTop(&panel.root)?;
        let x = (anchor.left + anchor.right) / 2;
        let left = (x - (outer_w / 2.0) as i32).clamp(monitor.left, (monitor.right - outer_w as i32).max(monitor.left));
        let top = if below { anchor.bottom } else { anchor.top - outer_h as i32 };
        let top = top.clamp(monitor.top, (monitor.bottom - outer_h as i32).max(monitor.top));
        self.bounds = RECT { left, top, right: left + outer_w as i32, bottom: top + outer_h as i32 };
        self.popup.show(self.bounds, false);
        self.open = true;
        Ok(())
    }

    pub fn show_tip(&mut self, c: &Compositor, gfx: &Gfx, key: u64) {
        let Some(text) = self.tip(key).map(str::to_string) else { return };
        let text: String = text.lines().next().unwrap_or_default().chars().take(70).collect();
        if text.trim().is_empty() { return; }
        let Some(header) = self.header.as_ref() else { return };
        let style = TextStyle {
            px: 12.0 * self.scale,
            weight: 600,
            color: ui::theme::text_2(),
            max_w: Some(self.header_max_w),
            ..Default::default()
        };
        let Ok((surface, width, height)) = gfx.text(&text, &style) else { return };
        let Ok(brush) = c.CreateSurfaceBrushWithSurface(&surface) else { return };
        let _ = header.SetBrush(&brush);
        let _ = header.SetSize(ui::v2(width, height));
        let _ = header.SetOffset(ui::v3(self.header_x, self.header_y, 0.0));
        let _ = header.SetOpacity(1.0);
    }

    pub fn reset_header(&mut self, c: &Compositor, gfx: &Gfx) {
        let Some(header) = self.header.as_ref() else { return };
        let style = TextStyle {
            px: 12.0 * self.scale,
            weight: 600,
            color: ui::theme::text_2(),
            max_w: Some(self.header_max_w),
            ..Default::default()
        };
        let Ok((surface, width, height)) = gfx.text(&self.header_home, &style) else { return };
        let Ok(brush) = c.CreateSurfaceBrushWithSurface(&surface) else { return };
        let _ = header.SetBrush(&brush);
        let _ = header.SetSize(ui::v2(width, height));
        let _ = header.SetOffset(ui::v3(self.header_x, self.header_y, 0.0));
        let _ = header.SetOpacity(1.0);
    }

    pub fn set_drop_target(&mut self, key: Option<u64>) {
        for cell in &self.cells {
            let _ = cell.background.SetOpacity(if Some(cell.key) == key { 1.0 } else if Some(cell.key) == self.hover { 0.55 } else { 0.0 });
        }
    }

    pub fn hide(&mut self) {
        self.open = false;
        self.hover = None;
        self.cells.clear();
        self.header = None;
        self.popup.hide();
        self.bounds = RECT::default();
    }
}

fn inside(r: RECT, x: f32, y: f32) -> bool {
    x >= r.left as f32 && x < r.right as f32 && y >= r.top as f32 && y < r.bottom as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(key: u64, stable_id: &str, app_hidden: bool) -> TrayIconView {
        TrayIconView { key, stable_id: stable_id.into(), tip: String::new(), brush: None, app_hidden, standard_tip: true }
    }

    #[test]
    fn partition_keeps_saved_order_and_sends_hidden_and_excess_to_overflow() {
        let icons = vec![icon(1, "a", false), icon(2, "b", false), icon(3, "c", true), icon(4, "d", false)];
        let order = vec!["d".into(), "b".into(), "a".into(), "c".into()];
        let (bar, overflow) = partition(&icons, &order, &[], 2);
        assert_eq!(bar.iter().map(|i| i.stable_id.as_str()).collect::<Vec<_>>(), vec!["d", "b"]);
        assert_eq!(overflow.iter().map(|i| i.stable_id.as_str()).collect::<Vec<_>>(), vec!["a"]);
    }

    #[test]
    fn app_hidden_icons_are_not_shown_even_in_overflow() {
        let icons = vec![icon(1, "a", true), icon(2, "b", false)];
        let (bar, overflow) = partition(&icons, &[], &["a".to_string()], 0);
        assert!(bar.is_empty());
        assert_eq!(overflow.iter().map(|i| i.stable_id.as_str()).collect::<Vec<_>>(), vec!["b"]);
    }

    #[test]
    fn user_hidden_icons_remain_in_overflow_even_when_the_bar_has_room() {
        let icons = vec![icon(1, "a", false), icon(2, "b", false)];
        let hidden = vec!["a".to_string()];
        let (bar, overflow) = partition(&icons, &[], &hidden, MAX_ON_BAR);
        assert_eq!(bar.iter().map(|i| i.stable_id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        assert_eq!(overflow.iter().map(|i| i.stable_id.as_str()).collect::<Vec<_>>(), vec!["a"]);
    }

    #[test]
    fn drag_updates_order_without_duplicates() {
        let mut order = vec!["a".into(), "b".into(), "c".into()];
        move_before(&mut order, "c", "a", false);
        assert_eq!(order, vec!["c".to_string(), "a".to_string(), "b".to_string()]);
        move_before(&mut order, "a", "b", true);
        assert_eq!(order, vec!["c".to_string(), "b".to_string(), "a".to_string()]);
    }

    #[test]
    fn drag_can_be_placed_after_a_new_icon_not_yet_in_saved_order() {
        let mut order = vec!["known".into()];
        move_before(&mut order, "moving", "new", true);
        assert_eq!(order, vec!["known".to_string(), "new".to_string(), "moving".to_string()]);
    }
}
