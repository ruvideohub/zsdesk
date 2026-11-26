//! Common visual language for desktop cards: the same live glass and hairline as Launchpad.
use crate::gfx::{Gfx, TextStyle};
use crate::ui::{self, theme, v2, v3};
use windows::core::{Interface, Result};
use windows::UI::Composition::{CompositionBrush, Compositor, ContainerVisual};

pub fn backdrop(c: &Compositor) -> Option<CompositionBrush> {
    c.CreateHostBackdropBrush()
        .ok()
        .and_then(|brush| brush.cast().ok())
}

/// The background has a deliberately opaque fallback if host backdrop is unavailable.
pub fn surface(
    c: &Compositor,
    root: &ContainerVisual,
    backdrop: Option<&CompositionBrush>,
    w: f32,
    h: f32,
    k: f32,
) -> Result<()> {
    let radius = (f32::from(aura_core::desktop_style::CORNER_RADIUS_DIP) * k).min(w.min(h) / 2.0);
    let environment = crate::desktop_card_policy::Environment {
        light: theme::light(),
        high_contrast: theme::high_contrast(),
        transparency: theme::transparency(),
        system_colors: [0; 3],
    };
    let glass = if environment.glass(backdrop.is_some()) {
        backdrop.and_then(|b| ui::glass(c, b, theme::surface(0.72), radius).ok())
    } else {
        None
    };
    if let Some((body, geometry)) = glass {
        body.SetSize(v2(w, h))?;
        geometry.SetSize(v2(w, h))?;
        root.Children()?.InsertAtTop(&body)?;
    } else {
        let (body, _, _) = ui::rounded(c, w, h, radius, theme::surface(1.0))?;
        root.Children()?.InsertAtTop(&body)?;
    }
    let (rim, _, shape) = ui::rounded(c, w - k, h - k, radius, ui::rgba(0, 0, 0, 0.0))?;
    rim.SetOffset(v3(k / 2.0, k / 2.0, 0.0))?;
    shape.SetStrokeBrush(&c.CreateColorBrushWithColor(theme::ink(0.20))?)?;
    shape.SetStrokeThickness(k.max(1.0))?;
    root.Children()?.InsertAtTop(&rim)?;
    // A short top highlight adds depth without extending beyond the rounded
    // surface or changing the card's click and placement geometry.
    if !theme::high_contrast() && w > radius * 2.0 {
        rect(
            c,
            root,
            radius,
            1.0 * k,
            w - radius * 2.0,
            k.max(1.0),
            0.0,
            theme::ink(0.08),
        )?;
    }
    Ok(())
}

pub fn rect(
    c: &Compositor,
    root: &ContainerVisual,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    color: windows::UI::Color,
) -> Result<()> {
    let (v, _, _) = ui::rounded(c, w, h, r, color)?;
    v.SetOffset(v3(x, y, 0.0))?;
    root.Children()?.InsertAtTop(&v)?;
    Ok(())
}

/// Quiet inset panel: faint fill and rim, with an opaque system-colour outline
/// instead of a translucent tint when Windows high contrast is active.
pub fn tile(
    c: &Compositor,
    root: &ContainerVisual,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    k: f32,
) -> Result<()> {
    let (v, _, shape) = ui::rounded(c, w, h, r, theme::ink(0.065))?;
    v.SetOffset(v3(x, y, 0.0))?;
    shape.SetStrokeBrush(&c.CreateColorBrushWithColor(theme::ink(
        if theme::high_contrast() { 0.20 } else { 0.08 },
    ))?)?;
    shape.SetStrokeThickness(k.max(1.0))?;
    root.Children()?.InsertAtTop(&v)?;
    Ok(())
}

/// Draw a bounded meter. Zero or invalid readings leave the track unfilled.
pub fn meter(
    c: &Compositor,
    root: &ContainerVisual,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    fill: f32,
) -> Result<()> {
    if theme::high_contrast() {
        tile(c, root, x, y, w, h, h / 2.0, 1.0)?;
    } else {
        rect(c, root, x, y, w, h, h / 2.0, theme::ink(0.11))?;
    }
    let color = if theme::high_contrast() {
        theme::ink(0.20)
    } else {
        theme::accent()
    };
    if let Some(width) = crate::desktop_card_policy::meter_width(fill, w, h) {
        rect(c, root, x, y, width, h, h / 2.0, color)?;
    }
    Ok(())
}

/// Render inside a fixed width; DirectWrite truncates instead of overlapping a neighbour.
pub fn text(
    c: &Compositor,
    gfx: &Gfx,
    root: &ContainerVisual,
    value: &str,
    x: f32,
    y: f32,
    max_w: f32,
    px: f32,
    weight: u32,
    color: (f32, f32, f32, f32),
    display: bool,
    right: bool,
) -> Result<()> {
    let (v, width, _) = ui::text(
        c,
        gfx,
        value,
        &TextStyle {
            px,
            weight,
            color,
            display,
            max_w: Some(max_w.max(1.0)),
            ..Default::default()
        },
    )?;
    v.SetOffset(v3(
        if right {
            x + (max_w - width).max(0.0)
        } else {
            x
        },
        y,
        0.0,
    ))?;
    root.Children()?.InsertAtTop(&v)?;
    Ok(())
}

/// Measure once and shrink before applying the final ellipsis-constrained width.
/// Also returns the actual visual width for adjacent elements such as AM/PM.
pub fn fitted_visual(
    c: &Compositor,
    gfx: &Gfx,
    value: &str,
    max_w: f32,
    style: &TextStyle,
) -> Result<(windows::UI::Composition::SpriteVisual, f32, f32)> {
    let mut natural_style = style.clone();
    natural_style.max_w = None;
    let (natural, width, height) = ui::text(c, gfx, value, &natural_style)?;
    if width <= max_w {
        return Ok((natural, width, height));
    }
    let mut fitted = natural_style;
    fitted.px = crate::desktop_card_policy::fitted_font_size(style.px, width, max_w);
    fitted.max_w = Some((max_w - 2.0).max(1.0));
    ui::text(c, gfx, value, &fitted)
}

/// Keep live numbers whole where possible; shrink before resorting to ellipsis.
pub fn fitted_text(
    c: &Compositor,
    gfx: &Gfx,
    root: &ContainerVisual,
    value: &str,
    x: f32,
    y: f32,
    max_w: f32,
    px: f32,
    weight: u32,
    color: (f32, f32, f32, f32),
    display: bool,
) -> Result<()> {
    let st = TextStyle { px, weight, color, display, ..Default::default() };
    let (visual, _, _) = fitted_visual(c, gfx, value, max_w, &st)?;
    visual.SetOffset(v3(x, y, 0.0))?;
    root.Children()?.InsertAtTop(&visual)?;
    Ok(())
}

/// A compact tinted icon tile and two-line header, shared by all information cards.
pub fn header(
    c: &Compositor,
    gfx: &Gfx,
    root: &ContainerVisual,
    title: &str,
    subtitle: &str,
    glyph: &str,
    k: f32,
) -> Result<()> {
    if theme::high_contrast() {
        tile(c, root, 18.0 * k, 16.0 * k, 32.0 * k, 32.0 * k, 11.0 * k, k)?;
    } else {
        rect(
            c,
            root,
            18.0 * k,
            16.0 * k,
            32.0 * k,
            32.0 * k,
            11.0 * k,
            ui::rgba(10, 132, 255, if theme::light() { 0.11 } else { 0.18 }),
        )?;
    }
    let icon_color = if theme::high_contrast() {
        theme::text()
    } else {
        theme::accent_f()
    };
    let (icon, iw, ih) = ui::text(
        c,
        gfx,
        glyph,
        &TextStyle {
            px: 15.0 * k,
            icons: true,
            color: icon_color,
            ..Default::default()
        },
    )?;
    icon.SetOffset(v3(34.0 * k - iw / 2.0, 32.0 * k - ih / 2.0, 0.0))?;
    root.Children()?.InsertAtTop(&icon)?;
    text(
        c,
        gfx,
        root,
        title,
        60.0 * k,
        14.0 * k,
        186.0 * k,
        14.0 * k,
        600,
        theme::text(),
        false,
        false,
    )?;
    text(
        c,
        gfx,
        root,
        subtitle,
        60.0 * k,
        33.0 * k,
        186.0 * k,
        10.0 * k,
        500,
        theme::text_2(),
        false,
        false,
    )?;
    Ok(())
}
