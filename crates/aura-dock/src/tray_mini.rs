//! A small, non-activating notification strip at the lower right of the main display.
use windows::core::Result;
use windows::UI::Composition::{CompositionBrush, Compositor, ShapeVisual};
use windows::Win32::Foundation::{HWND, RECT};
use crate::gfx::{Gfx, TextStyle};
use crate::tray_mini_layout as layout;
use crate::tray_strip::TrayIconView;
use crate::ui::{self, Popup};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit { Icon(u64), Overflow, Control }

pub struct MiniTray {
    popup: Popup,
    tooltip: Popup,
    pub visible: bool,
    bounds: RECT,
    cells: Vec<(RECT, Hit)>,
    highlights: Vec<(Hit, ShapeVisual)>,
}
impl MiniTray {
    pub fn new(c: &Compositor) -> Result<Self> {
        Ok(Self { popup: Popup::new(c, false)?, tooltip: Popup::new(c, false)?, visible: false, bounds: RECT::default(), cells: Vec::new(), highlights: Vec::new() })
    }
    pub fn hwnd(&self) -> HWND { self.popup.hwnd }
    pub fn bounds(&self) -> RECT { self.bounds }
    pub fn at(&self, x: f32, y: f32) -> Option<Hit> {
        self.cells.iter().find(|(r, _)| x >= r.left as f32 && x < r.right as f32 && y >= r.top as f32 && y < r.bottom as f32).map(|(_, hit)| *hit)
    }
    pub fn overflow_rect(&self) -> Option<RECT> {
        self.cells.iter().find(|(_, h)| *h == Hit::Overflow).map(|(r, _)| RECT {
            left: self.bounds.left + r.left, top: self.bounds.top + r.top,
            right: self.bounds.left + r.right, bottom: self.bounds.top + r.bottom,
        })
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.visible && x >= self.bounds.left && x < self.bounds.right && y >= self.bounds.top && y < self.bounds.bottom
    }
    pub fn hide_tip(&self) { self.tooltip.hide(); }
    pub fn hide(&mut self) { self.visible = false; self.hide_tip(); self.popup.hide(); }
    pub fn show_tip(&self, c: &Compositor, gfx: &Gfx, backdrop: &CompositionBrush, key: u64, text: &str, monitor: RECT, scale: f32) -> Result<()> {
        let Some((cell, _)) = self.cells.iter().find(|(_, hit)| *hit == Hit::Icon(key)) else { return Ok(()) };
        let label: String = text.lines().next().unwrap_or_default().chars().take(100).collect();
        if label.trim().is_empty() { return Ok(()); }
        let style = TextStyle { px: 12.0*scale, color: ui::theme::text(), max_w: Some(270.0*scale), ..Default::default() };
        let (sprite, w, h) = ui::text(c, gfx, &label, &style)?;
        let pad = 8.0*scale;
        let width = (w + 2.0*pad).ceil() as i32;
        let height = (h + 2.0*pad).ceil() as i32;
        let rightmost = (monitor.right-width).max(monitor.left);
        let left = (self.bounds.left + cell.left + (cell.right-cell.left-width)/2).clamp(monitor.left, rightmost);
        let top = (self.bounds.top-height-(5.0*scale) as i32).max(monitor.top);
        let root = &self.tooltip.root;
        root.Children()?.RemoveAll()?;
        root.SetSize(ui::v2(width as f32, height as f32))?;
        let (glass, shape) = ui::glass(c, backdrop, ui::theme::surface(0.92), 7.0*scale)?;
        glass.SetSize(ui::v2(width as f32, height as f32))?;
        shape.SetSize(ui::v2(width as f32, height as f32))?;
        root.Children()?.InsertAtTop(&glass)?;
        sprite.SetOffset(ui::v3(pad, pad, 0.0))?;
        root.Children()?.InsertAtTop(&sprite)?;
        self.tooltip.show(RECT { left, top, right: left+width, bottom: top+height }, false);
        Ok(())
    }
    pub fn set_hover(&self, hit: Option<Hit>) {
        for (item, visual) in &self.highlights { let _ = visual.SetOpacity(if Some(*item) == hit { 0.6 } else { 0.0 }); }
    }
    pub fn show(&mut self, c: &Compositor, gfx: &Gfx, backdrop: &CompositionBrush, icons: &[TrayIconView], overflow: bool, screen: RECT, scale: f32) -> Result<()> {
        let s = scale.max(0.25);
        let width = layout::width(icons.len(), overflow, s).min((screen.right - screen.left).max(1));
        let height = (layout::HEIGHT * s).ceil() as i32;
        let margin = (8.0 * s).round() as i32;
        let right = (screen.right - margin).max(screen.left + width);
        let bottom = (screen.bottom - margin).max(screen.top + height);
        self.bounds = RECT { left: right - width, top: bottom - height, right, bottom };
        self.hide_tip();
        self.cells.clear();
        self.highlights.clear();
        let root = &self.popup.root;
        root.Children()?.RemoveAll()?;
        root.SetSize(ui::v2(width as f32, height as f32))?;
        let (glass, shape) = ui::glass(c, backdrop, ui::theme::surface(0.82), 12.0 * s)?;
        glass.SetSize(ui::v2(width as f32, height as f32))?;
        shape.SetSize(ui::v2(width as f32, height as f32))?;
        root.Children()?.InsertAtTop(&glass)?;
        let mut x = layout::PAD * s;
        let slot = layout::SLOT * s;
        let h = height as f32;
        if overflow {
            self.cells.push((rect(x, 0.0, slot, h), Hit::Overflow));
            let (bg, _, _) = ui::rounded(c, slot, h-4.0*s, 7.0*s, ui::theme::fill_2())?;
            bg.SetOffset(ui::v3(x, 2.0*s, 0.0))?; bg.SetOpacity(0.0)?;
            root.Children()?.InsertAtTop(&bg)?; self.highlights.push((Hit::Overflow, bg));
            let (glyph, gw, gh) = ui::text(c, gfx, "\u{E70E}", &TextStyle { icons: true, px: 12.0*s, color: ui::theme::text(), ..Default::default() })?;
            glyph.SetOffset(ui::v3(x + (slot-gw)/2.0, (h-gh)/2.0, 0.0))?;
            root.Children()?.InsertAtTop(&glyph)?;
            x += slot;
        }
        for icon in icons {
            self.cells.push((rect(x, 0.0, slot, h), Hit::Icon(icon.key)));
            let (bg, _, _) = ui::rounded(c, slot, h-4.0*s, 7.0*s, ui::theme::fill_2())?;
            bg.SetOffset(ui::v3(x, 2.0*s, 0.0))?; bg.SetOpacity(0.0)?;
            root.Children()?.InsertAtTop(&bg)?; self.highlights.push((Hit::Icon(icon.key), bg));
            if let Some(brush) = &icon.brush {
                let visual = c.CreateSpriteVisual()?;
                visual.SetBrush(brush)?;
                let size = 16.0 * s;
                visual.SetSize(ui::v2(size, size))?;
                visual.SetOffset(ui::v3(x + (slot-size)/2.0, (h-size)/2.0, 0.0))?;
                root.Children()?.InsertAtTop(&visual)?;
            }
            x += slot;
        }
        let cw = layout::CONTROL * s;
        self.cells.push((rect(x, 0.0, cw, h), Hit::Control));
        let (bg, _, _) = ui::rounded(c, cw, h-4.0*s, 7.0*s, ui::theme::fill_2())?;
        bg.SetOffset(ui::v3(x, 2.0*s, 0.0))?; bg.SetOpacity(0.0)?;
        root.Children()?.InsertAtTop(&bg)?; self.highlights.push((Hit::Control, bg));
        let (glyph, gw, gh) = ui::text(c, gfx, "\u{E701} \u{E767}", &TextStyle { icons: true, px: 12.0*s, color: ui::theme::text(), ..Default::default() })?;
        glyph.SetOffset(ui::v3(x + (cw-gw)/2.0, (h-gh)/2.0, 0.0))?;
        root.Children()?.InsertAtTop(&glyph)?;
        self.popup.show(self.bounds, false);
        self.visible = true;
        Ok(())
    }
}
fn rect(x: f32, y: f32, width: f32, height: f32) -> RECT {
    RECT { left: x.floor() as i32, top: y.floor() as i32, right: (x+width).ceil() as i32, bottom: (y+height).ceil() as i32 }
}
