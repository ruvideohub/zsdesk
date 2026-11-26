//! Common visual language for desktop cards: the same live glass and hairline as Launchpad.
use windows::core::{Interface, Result};
use windows::UI::Composition::{CompositionBrush, Compositor, ContainerVisual};
use crate::gfx::{Gfx, TextStyle};
use crate::ui::{self, theme, v2, v3};

pub fn backdrop(c: &Compositor) -> Option<CompositionBrush> {
    c.CreateHostBackdropBrush().ok().and_then(|brush| brush.cast().ok())
}

/// The background has a deliberately opaque fallback if host backdrop is unavailable.
pub fn surface(c: &Compositor, root: &ContainerVisual, backdrop: Option<&CompositionBrush>, w: f32, h: f32, k: f32) -> Result<()> {
    let radius = f32::from(aura_core::desktop_style::CORNER_RADIUS_DIP) * k;
    let glass = backdrop.and_then(|b| ui::glass(c, b, theme::surface(0.72), radius).ok());
    if let Some((body, geometry)) = glass {
        body.SetSize(v2(w, h))?;
        geometry.SetSize(v2(w, h))?;
        root.Children()?.InsertAtTop(&body)?;
    } else {
        let (body, _, _) = ui::rounded(c, w, h, radius, theme::surface(0.96))?;
        root.Children()?.InsertAtTop(&body)?;
    }
    let (rim, _, shape) = ui::rounded(c, w - k, h - k, radius, ui::rgba(0, 0, 0, 0.0))?;
    rim.SetOffset(v3(k / 2.0, k / 2.0, 0.0))?;
    shape.SetStrokeBrush(&c.CreateColorBrushWithColor(theme::ink(0.20))?)?;
    shape.SetStrokeThickness(k.max(1.0))?;
    root.Children()?.InsertAtTop(&rim)?;
    Ok(())
}

pub fn rect(c: &Compositor, root: &ContainerVisual, x: f32, y: f32, w: f32, h: f32, r: f32, color: windows::UI::Color) -> Result<()> {
    let (v, _, _) = ui::rounded(c, w, h, r, color)?;
    v.SetOffset(v3(x, y, 0.0))?;
    root.Children()?.InsertAtTop(&v)?;
    Ok(())
}

/// Render inside a fixed width; DirectWrite truncates instead of overlapping a neighbour.
pub fn text(c: &Compositor, gfx: &Gfx, root: &ContainerVisual, value: &str, x: f32, y: f32, max_w: f32, px: f32, weight: u32, color: (f32,f32,f32,f32), display: bool, right: bool) -> Result<()> {
    let (v, width, _) = ui::text(c, gfx, value, &TextStyle { px, weight, color, display, max_w: Some(max_w.max(1.0)), ..Default::default() })?;
    v.SetOffset(v3(if right { x + (max_w - width).max(0.0) } else { x }, y, 0.0))?;
    root.Children()?.InsertAtTop(&v)?;
    Ok(())
}

/// Keep live numbers whole where possible; shrink before resorting to ellipsis.
pub fn fitted_text(c: &Compositor, gfx: &Gfx, root: &ContainerVisual, value: &str, x: f32, y: f32, max_w: f32, px: f32, weight: u32, color: (f32,f32,f32,f32), display: bool) -> Result<()> {
    let st = TextStyle { px, weight, color, display, ..Default::default() };
    let (natural, width, _) = ui::text(c, gfx, value, &st)?;
    if width <= max_w {
        natural.SetOffset(v3(x, y, 0.0))?;
        root.Children()?.InsertAtTop(&natural)?;
        Ok(())
    } else {
        let smaller = (px * (max_w - 3.0).max(1.0) / width).max(px * 0.70);
        text(c, gfx, root, value, x, y, max_w, smaller, weight, color, display, false)
    }
}

/// A compact tinted icon tile and two-line header, shared by all information cards.
pub fn header(c: &Compositor, gfx: &Gfx, root: &ContainerVisual, title: &str, subtitle: &str, glyph: &str, k: f32) -> Result<()> {
    rect(c, root, 18.0*k, 16.0*k, 32.0*k, 32.0*k, 11.0*k, ui::rgba(10,132,255,if theme::light() {0.11} else {0.18}))?;
    let (icon, iw, ih) = ui::text(c, gfx, glyph, &TextStyle { px: 15.0*k, icons: true, color: theme::accent_f(), ..Default::default() })?;
    icon.SetOffset(v3(34.0*k-iw/2.0, 32.0*k-ih/2.0, 0.0))?;
    root.Children()?.InsertAtTop(&icon)?;
    text(c, gfx, root, title, 60.0*k, 14.0*k, 186.0*k, 14.0*k, 600, theme::text(), false, false)?;
    text(c, gfx, root, subtitle, 60.0*k, 33.0*k, 186.0*k, 10.0*k, 500, theme::text_2(), false, false)?;
    Ok(())
}
