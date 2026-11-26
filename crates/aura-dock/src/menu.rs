//! Styled context menus: a glass panel with a soft shadow, an accent highlight
//! that glides between rows, check marks, shortcuts, separators and headers.
//! Mouse and keyboard (Up/Down/Enter/Esc). Closes when it loses focus.

use windows::core::{Interface, Result};
use windows::UI::Composition::*;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SPACE, VK_UP};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

use crate::gfx::{Gfx, TextStyle};
use crate::ui::{self, theme, v2, v3, Popup};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Item,
    Separator,
    Header,
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub kind: Kind,
    pub label: String,
    pub shortcut: Option<String>,
    pub cmd: usize,
    pub checked: bool,
    pub enabled: bool,
    pub danger: bool,
    pub children: Vec<MenuItem>,
    pub(crate) back: bool,
}

impl MenuItem {
    pub fn item(label: impl Into<String>, cmd: usize) -> Self {
        let label = crate::strings::tr_owned(label.into());
        Self { kind: Kind::Item, label, shortcut: None, cmd, checked: false, enabled: true, danger: false, children: Vec::new(), back: false }
    }
    pub fn submenu(label: impl Into<String>, children: Vec<MenuItem>) -> Self {
        Self { children, ..Self::item(label, 0) }
    }
    fn back() -> Self {
        Self { back: true, ..Self::item("Back", 0) }
    }
    pub fn sep() -> Self {
        Self { kind: Kind::Separator, ..Self::item("", 0) }
    }
    pub fn header(label: impl Into<String>) -> Self {
        Self { kind: Kind::Header, ..Self::item(label, 0) }
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into());
        self
    }
}

#[derive(Debug)]
pub enum MenuAction { Command(usize), Open(Vec<MenuItem>), Back, Close }

fn action_for(item: &MenuItem) -> MenuAction {
    if item.back { MenuAction::Back }
    else if !item.children.is_empty() { MenuAction::Open(item.children.clone()) }
    else { MenuAction::Command(item.cmd) }
}

struct Row {
    y: f32,
    h: f32,
    item: usize,
}

struct MenuFrame {
    items: Vec<MenuItem>,
    scroll_y: f32,
    hover: Option<usize>,
}

pub struct Menu {
    pub popup: Popup,
    items: Vec<MenuItem>,
    rows: Vec<Row>,
    highlight: Option<ShapeVisual>,
    hover: Option<usize>,
    origin: (f32, f32), // panel top-left inside the window
    panel_w: f32,
    panel_h: f32,
    content_h: f32,
    scroll_y: f32,
    scroll_inner: Option<ContainerVisual>,
    scroll_thumb: Option<ShapeVisual>,
    scale: f32,
    restore_focus: HWND,
    trail: Vec<MenuFrame>,
    anchor: POINT,
    above: bool,
    monitor: RECT,
    pub open: bool,
}

impl Menu {
    pub fn new(c: &Compositor) -> Result<Self> {
        Ok(Self {
            popup: Popup::new(c, true)?,
            items: Vec::new(),
            rows: Vec::new(),
            highlight: None,
            hover: None,
            origin: (0.0, 0.0),
            panel_w: 0.0,
            panel_h: 0.0,
            content_h: 0.0,
            scroll_y: 0.0,
            scroll_inner: None,
            scroll_thumb: None,
            scale: 1.0,
            restore_focus: HWND::default(),
            trail: Vec::new(),
            anchor: POINT::default(),
            above: false,
            monitor: RECT::default(),
            open: false,
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.popup.hwnd
    }

    /// Show at `anchor` (screen px). `above`: open upwards from the anchor (dock menus).
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        c: &Compositor,
        gfx: &Gfx,
        backdrop: &CompositionBrush,
        items: Vec<MenuItem>,
        anchor: POINT,
        above: bool,
        scale: f32,
        monitor: RECT,
    ) -> Result<()> {
        self.trail.clear();
        self.show_view(c, gfx, backdrop, items, anchor, above, scale, monitor)
    }

    #[allow(clippy::too_many_arguments)]
    fn show_view(
        &mut self, c: &Compositor, gfx: &Gfx, backdrop: &CompositionBrush,
        items: Vec<MenuItem>, anchor: POINT, above: bool, scale: f32, monitor: RECT,
    ) -> Result<()> {
        if !self.open {
            self.restore_focus = unsafe { GetForegroundWindow() };
        }
        self.anchor = anchor;
        self.above = above;
        self.monitor = monitor;
        let s = scale;
        self.scale = s;
        self.items = items;
        self.rows.clear();
        self.hover = None;
        let root = &self.popup.root;
        root.Children()?.RemoveAll()?;

        // Measure and render text first.
        let pad = 5.0 * s;
        let row_h = 26.0 * s;
        let sep_h = 9.0 * s;
        let head_h = 22.0 * s;
        let text_x = 26.0 * s;
        // `ui::panel` reserves 22 DIP blur plus 6 DIP offset on each side.
        let shadow_margin = (28.0 * s).ceil();
        let max_body_w = crate::menu_layout::viewport(f32::MAX, (monitor.right - monitor.left).max(1) as f32, shadow_margin);
        let label_limit = (max_body_w - text_x - 2.0 * pad - 48.0 * s).clamp(1.0, 300.0 * s);
        let mut labels = Vec::new();
        let mut max_w: f32 = 200.0 * s;
        for it in &self.items {
            match it.kind {
                Kind::Separator => labels.push(None),
                _ => {
                    let col = if !it.enabled || it.kind == Kind::Header {
                        theme::text_3()
                    } else if it.danger {
                        theme::DANGER
                    } else {
                        theme::text()
                    };
                    let weight = if it.kind == Kind::Header { 600 } else { 400 };
                    let px = if it.kind == Kind::Header { 11.5 } else { 13.0 } * s;
                    let (lab, lw, lh) = ui::text(c, gfx, &it.label, &TextStyle { px, weight, color: col, max_w: Some(label_limit), ..Default::default() })?;
                    let arrow = if !it.children.is_empty() { Some("›") } else if it.back { Some("‹") } else { None };
                    let sc = match arrow.or(it.shortcut.as_deref()) {
                        Some(t) => Some(ui::text(c, gfx, t, &TextStyle { px: 12.0 * s, color: theme::text_3(), ..Default::default() })?),
                        None => None,
                    };
                    let sw = sc.as_ref().map(|x| x.1 + 24.0 * s).unwrap_or(0.0);
                    max_w = max_w.max(text_x + lw + sw + 14.0 * s);
                    labels.push(Some((lab, lw, lh, sc)));
                }
            }
        }
        let w = (max_w + 2.0 * pad).ceil().min(max_body_w);
        let mut y = pad;
        for (i, it) in self.items.iter().enumerate() {
            let h = match it.kind {
                Kind::Separator => sep_h,
                Kind::Header => head_h,
                Kind::Item => row_h,
            };
            self.rows.push(Row { y, h, item: i });
            y += h;
        }
        let content_h = (y + pad).ceil();
        let h = crate::menu_layout::viewport(content_h, (monitor.bottom - monitor.top).max(1) as f32, shadow_margin);
        self.panel_h = h;
        self.content_h = content_h;
        self.scroll_y = 0.0;
        self.scroll_thumb = None;

        let radius = 12.0 * s;
        let panel = ui::panel(c, gfx, backdrop, w, h, radius, theme::surface(0.74), s, 0.42)?;
        let m = panel.margin;
        self.origin = (m, m);
        self.panel_w = w;
        // All rows move together underneath an explicit clipped viewport. The background,
        // rim and shadow stay fixed, and hit testing uses the same scroll offset.
        panel.content.SetClip(&c.CreateInsetClip()?)?;
        let inner = c.CreateContainerVisual()?;
        inner.SetSize(v2(w, content_h))?;
        panel.content.Children()?.InsertAtTop(&inner)?;
        self.scroll_inner = Some(inner.clone());

        // Hover highlight (moves between rows).
        let (hl, _, _) = ui::rounded(c, (w - 2.0 * pad).max(1.0), row_h, 6.0 * s, theme::accent())?;
        hl.SetOpacity(0.0)?;
        hl.SetOffset(v3(pad, pad, 0.0))?;
        inner.Children()?.InsertAtTop(&hl)?;
        self.highlight = Some(hl);

        for (row, lab) in self.rows.iter().zip(labels) {
            let it = &self.items[row.item];
            match (it.kind, lab) {
                (Kind::Separator, _) => {
                    let (line, _, _) = ui::rounded(c, (w - 2.0 * pad - 16.0 * s).max(1.0), 1.0, 0.0, theme::hair())?;
                    line.SetOffset(v3(pad + 8.0 * s, row.y + (row.h / 2.0).floor(), 0.0))?;
                    inner.Children()?.InsertAtTop(&line)?;
                }
                (_, Some((lab, _lw, lh, sc))) => {
                    let x = if it.kind == Kind::Header { pad + 10.0 * s } else { pad + text_x };
                    lab.SetOffset(v3(x, (row.y + (row.h - lh) / 2.0).round(), 0.0))?;
                    inner.Children()?.InsertAtTop(&lab)?;
                    if it.checked {
                        let (ck, _, ch) = ui::text(c, gfx, "\u{2713}", &TextStyle { px: 13.0 * s, weight: 600, ..Default::default() })?;
                        ck.SetOffset(v3(pad + 9.0 * s, (row.y + (row.h - ch) / 2.0).round(), 0.0))?;
                        inner.Children()?.InsertAtTop(&ck)?;
                    }
                    if let Some((sv, sw, sh)) = sc {
                        sv.SetOffset(v3(w - pad - 12.0 * s - sw, (row.y + (row.h - sh) / 2.0).round(), 0.0))?;
                        inner.Children()?.InsertAtTop(&sv)?;
                    }
                }
                _ => {}
            }
        }
        if content_h > h {
            let (top, height) = crate::menu_layout::thumb(h, content_h, 0.0, pad, 16.0 * s);
            let (thumb, _, _) = ui::rounded(c, 3.0 * s, height, 1.5 * s, theme::ink(0.34))?;
            thumb.SetOffset(v3((w - 5.0 * s).max(0.0), top, 0.0))?;
            panel.content.Children()?.InsertAtTop(&thumb)?;
            self.scroll_thumb = Some(thumb);
        }
        root.Children()?.InsertAtTop(&panel.root)?;

        // Position: next to the anchor, kept on screen.
        let (ww, wh) = ((w + 2.0 * m).ceil() as i32, (h + 2.0 * m).ceil() as i32);
        let left = anchor.x - m as i32;
        let top = if above { anchor.y - wh + m as i32 - (4.0 * s) as i32 } else { anchor.y - m as i32 };
        let r = ui::clamp_rect(RECT { left, top, right: left + ww, bottom: top + wh }, monitor);

        // Open animation: fade + slight scale from the anchor corner.
        let cx = (anchor.x - r.left) as f32 - m;
        let cy = if above { h } else { 0.0 };
        panel.body.SetCenterPoint(v3(cx, cy, 0.0))?;
        let body: CompositionObject = panel.body.cast()?;
        ui::animate_v3_from(c, &body, "Scale", v3(0.94, 0.94, 1.0), v3(1.0, 1.0, 1.0), 180)?;
        root.SetOpacity(0.0)?;
        ui::animate_scalar(c, &root.cast()?, "Opacity", 1.0, 110)?;

        self.popup.show(r, true);
        self.open = true;
        Ok(())
    }

    pub fn hide(&mut self, restore_focus: bool) {
        if !self.open {
            return;
        }
        self.open = false;
        self.scroll_inner = None;
        self.scroll_thumb = None;
        self.popup.hide();
        if restore_focus && !self.restore_focus.is_invalid() {
            unsafe {
                let _ = SetForegroundWindow(self.restore_focus);
            }
        }
    }

    fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        let (ox, oy) = self.origin;
        if x < ox || x >= ox + self.panel_w {
            return None;
        }
        let y = crate::menu_layout::hit_y(y, oy, self.panel_h, self.scroll_y)?;
        self.rows.iter().position(|r| y >= r.y && y < r.y + r.h)
    }

    fn set_scroll(&mut self, y: f32) {
        let max = crate::menu_layout::max_scroll(self.content_h, self.panel_h);
        self.scroll_y = y.clamp(0.0, max);
        if let Some(inner) = &self.scroll_inner { let _ = inner.SetOffset(v3(0.0, -self.scroll_y, 0.0)); }
        if let Some(thumb) = &self.scroll_thumb {
            let (top, _) = crate::menu_layout::thumb(self.panel_h, self.content_h, self.scroll_y, 5.0*self.scale, 16.0*self.scale);
            let _ = thumb.SetOffset(v3((self.panel_w - 5.0*self.scale).max(0.0), top, 0.0));
        }
    }

    /// Windows wheel deltas are multiples of 120, but precision wheels may send less.
    pub fn on_wheel(&mut self, c: &Compositor, delta: i16) {
        if crate::menu_layout::max_scroll(self.content_h, self.panel_h) <= 0.0 { return; }
        self.set_scroll(self.scroll_y - (delta as f32 / 120.0) * 3.0 * 26.0 * self.scale);
        // The old hover is no longer under the pointer after scrolling.
        self.set_hover(c, None);
    }

    fn reveal_row(&mut self, row: usize) {
        if let Some(r) = self.rows.get(row) {
            let y = crate::menu_layout::reveal(self.scroll_y, r.y, r.h, self.panel_h, self.content_h, 5.0*self.scale);
            self.set_scroll(y);
        }
    }

    fn selectable(&self, row: usize) -> bool {
        let it = &self.items[self.rows[row].item];
        it.kind == Kind::Item && it.enabled
    }

    fn set_hover(&mut self, c: &Compositor, row: Option<usize>) {
        let row = row.filter(|r| self.selectable(*r));
        if row == self.hover {
            return;
        }
        let first = self.hover.is_none();
        self.hover = row;
        let Some(hl) = &self.highlight else { return };
        let Ok(obj) = hl.cast::<CompositionObject>() else { return };
        match row {
            Some(r) => {
                let y = self.rows[r].y;
                let pad = 5.0 * self.scale;
                if first {
                    let _ = hl.SetOffset(v3(pad, y, 0.0));
                } else {
                    let _ = ui::animate_v3(c, &obj, "Offset", v3(pad, y, 0.0), 90);
                }
                let _ = ui::animate_scalar(c, &obj, "Opacity", 1.0, 60);
            }
            None => {
                let _ = ui::animate_scalar(c, &obj, "Opacity", 0.0, 90);
            }
        }
    }

    pub fn on_move(&mut self, c: &Compositor, x: f32, y: f32) {
        let r = self.row_at(x, y);
        self.set_hover(c, r);
    }

    /// A click opens a submenu, returns to its parent, or runs a command.
    pub fn on_click(&mut self, x: f32, y: f32) -> Option<MenuAction> {
        let r = self.row_at(x, y)?;
        if self.selectable(r) {
            Some(action_for(&self.items[self.rows[r].item]))
        } else {
            None
        }
    }

    pub fn navigate(&mut self, c: &Compositor, gfx: &Gfx, backdrop: &CompositionBrush, action: MenuAction) -> Result<()> {
        let (items, restore) = match action {
            MenuAction::Open(children) => {
                self.trail.push(MenuFrame { items: self.items.clone(), scroll_y: self.scroll_y, hover: self.hover });
                let mut items = vec![MenuItem::back(), MenuItem::sep()];
                items.extend(children);
                (items, None)
            }
            MenuAction::Back => match self.trail.pop() {
                Some(frame) => (frame.items, Some((frame.scroll_y, frame.hover))),
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
        self.show_view(c, gfx, backdrop, items, self.anchor, self.above, self.scale, self.monitor)?;
        if let Some((scroll_y, hover)) = restore {
            self.set_scroll(scroll_y);
            self.set_hover(c, hover);
        }
        Ok(())
    }

    /// Arrow keys navigate, Enter activates, Escape returns to the parent or closes.
    pub fn on_key(&mut self, c: &Compositor, vk: u16) -> Option<MenuAction> {
        let n = self.rows.len();
        let step = |from: Option<usize>, d: i32| -> Option<usize> {
            let mut i = from.map(|v| v as i32).unwrap_or(if d > 0 { -1 } else { n as i32 });
            for _ in 0..n {
                i = (i + d).rem_euclid(n as i32);
                if self.selectable(i as usize) {
                    return Some(i as usize);
                }
            }
            None
        };
        if vk == VK_DOWN.0 {
            let r = step(self.hover, 1);
            if let Some(row) = r { self.reveal_row(row); }
            self.set_hover(c, r);
            None
        } else if vk == VK_UP.0 {
            let r = step(self.hover, -1);
            if let Some(row) = r { self.reveal_row(row); }
            self.set_hover(c, r);
            None
        } else if vk == VK_HOME.0 || vk == VK_END.0 {
            let r = if vk == VK_HOME.0 { step(None, 1) } else { step(None, -1) };
            if let Some(row) = r { self.reveal_row(row); }
            self.set_hover(c, r);
            None
        } else if vk == VK_RETURN.0 || vk == VK_SPACE.0 {
            self.hover.map(|r| action_for(&self.items[self.rows[r].item]))
        } else if vk == VK_RIGHT.0 {
            self.hover.and_then(|r| { let item = &self.items[self.rows[r].item]; if item.children.is_empty() { None } else { Some(action_for(item)) } })
        } else if vk == VK_LEFT.0 {
            if self.trail.is_empty() { None } else { Some(MenuAction::Back) }
        } else if vk == VK_ESCAPE.0 {
            Some(if self.trail.is_empty() { MenuAction::Close } else { MenuAction::Back })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_item_actions_preserve_commands_and_submenus() {
        let child = MenuItem::item("Clock", 42).checked(true);
        let parent = MenuItem::submenu("Desktop Widgets", vec![child]);
        match action_for(&parent) {
            MenuAction::Open(children) => {
                assert_eq!(children.len(), 1);
                assert!(children[0].checked);
                assert!(matches!(action_for(&children[0]), MenuAction::Command(42)));
            }
            other => panic!("unexpected action: {other:?}"),
        }
        assert!(matches!(action_for(&MenuItem::back()), MenuAction::Back));
    }
}
