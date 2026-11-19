//! The control centre: one glass drop-down above the panel, split into sections.
//!
//! Top to bottom: the quick tiles (Wi-Fi, Bluetooth, airplane mode, night light, do not
//! disturb, dark mode), the three sliders (sound, microphone, brightness), the network
//! list with the networks in range, what is playing, and a footer with the battery and the
//! tray icons. It replaces hunting through the tray for those things.
//!
//! Everything is drawn into one Composition drawing surface with the widget canvas (so it
//! follows the light/dark theme and the display scale), and clicks come back as widget
//! actions ([`WAct`]), so the panel's existing handlers do the actual work.

use std::collections::HashMap;

use windows::core::{Interface, Result};
use windows::UI::Composition::*;
use windows::Win32::Foundation::{HWND, RECT};

use crate::gfx::Gfx;
use crate::services::ControlState;
use crate::ui::{self, theme, Popup};
use crate::widgets::canvas::{alpha, hex, Canvas, Col};
use crate::widgets::WAct;

// Segoe Fluent Icons.
const G_WIFI: &str = "\u{E701}";
const G_BT: &str = "\u{E702}";
const G_MOON: &str = "\u{E708}";
const G_SUN: &str = "\u{E706}";
const G_VOL: &str = "\u{E767}";
const G_MUTE: &str = "\u{E74F}";
const G_MIC: &str = "\u{E720}";
const G_PLAY: &str = "\u{E768}";
const G_PAUSE: &str = "\u{E769}";
const G_PREV: &str = "\u{E892}";
const G_NEXT: &str = "\u{E893}";
const G_DND: &str = "\u{E7ED}";
const G_SETTINGS: &str = "\u{E713}";
const G_CHEV: &str = "\u{E76C}";
const G_CHEV_UP: &str = "\u{E70E}";
const G_PLANE: &str = "\u{E709}";
const G_MUSIC: &str = "\u{E8D6}";
const G_LOCK: &str = "\u{E72E}";
const G_BATTERY: &str = "\u{E83F}";
const G_PLUG: &str = "\u{E83E}";

const BLUE: Col = hex(0x0A84FF);
const GREEN: Col = hex(0x30D158);
const PURPLE: Col = hex(0xBF5AF2);
const ORANGE: Col = hex(0xFF9F0A);
const YELLOW: Col = hex(0xFFD60A);
const WHITE: Col = (1.0, 1.0, 1.0, 1.0);

/// Drop-down width in logical px.
const W: f32 = 420.0;
const PAD: f32 = 18.0;
const RADIUS: f32 = 22.0;
const HEAD_H: f32 = 30.0;
const TILE_H: f32 = 62.0;
const TILE_GAP: f32 = 10.0;
const ROW_H: f32 = 46.0;
const NET_ROW_H: f32 = 34.0;
const PLAYER_H: f32 = 64.0;
const FOOT_H: f32 = 32.0;
const SEP_H: f32 = 19.0;
/// Networks listed under the network section.
const NET_SHOWN: usize = 5;

/// A rectangle in logical px: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

/// What a click in the control centre asks for.
#[derive(Clone, Debug)]
pub enum CAct {
    /// A widget action: the panel's own handlers do the work.
    Do(WAct),
    /// Show or hide the network list.
    Networks,
}

/// Everything the control centre draws, borrowed for one paint.
pub struct Data<'a> {
    pub control: &'a ControlState,
    pub wifi: Option<&'a std::result::Result<Vec<laminar_sys::radios::WifiNetwork>, u32>>,
    /// Tray icons, drawn by the panel on top of the bitmap: (key, tooltip).
    pub tray: &'a [(u64, String)],
    pub mic_muted: bool,
    pub brightness: Option<f32>,
    /// The display takes no DDC/CI brightness commands: the row shows a dash.
    pub brightness_unsupported: bool,
    pub night_light: bool,
    pub airplane: bool,
    pub dnd: bool,
    /// Battery: percent, charging, time left ("4 ч 12 мин", may be empty).
    pub battery: Option<(f32, bool, String)>,
}

struct Hit {
    rect: Rect,
    act: CAct,
}

pub struct ControlCenter {
    popup: Popup,
    pub open: bool,
    pub networks_open: bool,
    scale: f32,
    size: (f32, f32),
    surface: Option<(f32, f32, f32, CompositionDrawingSurface)>,
    hits: Vec<Hit>,
    /// The hovered area, as a rectangle (repainted as a soft highlight).
    hover: Option<Rect>,
    /// The slider being dragged: its whole clickable rectangle and action.
    drag: Option<(Rect, CAct)>,
    icons: Vec<(f32, f32, f32, String)>,
}

impl ControlCenter {
    pub fn new(c: &Compositor) -> Result<Self> {
        Ok(Self {
            popup: Popup::new(c, true)?,
            open: false,
            networks_open: false,
            scale: 1.0,
            size: (W, 420.0),
            surface: None,
            hits: Vec::new(),
            hover: None,
            drag: None,
            icons: Vec::new(),
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.popup.hwnd
    }

    // ------------------------------------------------------------------ layout

    fn network_rows(&self, d: &Data) -> usize {
        if !self.networks_open {
            return 0;
        }
        match d.wifi {
            Some(Ok(list)) => list.len().min(NET_SHOWN),
            _ => 1,
        }
    }

    /// The drop-down's height with the current content.
    fn height(&self, d: &Data) -> f32 {
        let tiles = TILE_H * 2.0 + TILE_GAP;
        let sliders = ROW_H * 3.0;
        let net = 30.0 + self.network_rows(d) as f32 * NET_ROW_H;
        PAD + HEAD_H + 10.0 + tiles + SEP_H + sliders + SEP_H + net + SEP_H + PLAYER_H + SEP_H + FOOT_H + PAD
    }

    // ------------------------------------------------------------------ open / close

    /// Show the drop-down right-aligned above the panel. `dock_top` is the panel's top edge
    /// in screen px.
    pub fn show(&mut self, c: &Compositor, gfx: &Gfx, backdrop: &CompositionBrush, monitor: RECT, dock_top: f32, scale: f32, d: &Data, tray_brushes: &HashMap<u64, (u64, CompositionSurfaceBrush)>) -> Result<()> {
        self.scale = scale;
        self.size = (W, self.height(d));
        let (w, h) = ((W * scale).ceil(), (self.size.1 * scale).ceil());
        let right = monitor.right - (12.0 * scale) as i32;
        let left = right - w as i32;
        let bottom = (dock_top - 8.0 * scale).round() as i32;
        let top = bottom - h as i32;
        self.render(c, gfx, backdrop, d, tray_brushes)?;
        self.popup.show(RECT { left, top, right, bottom }, true);
        // Rounded corners: the drop-down sits on a rounded card.
        unsafe {
            let r = (RADIUS * scale) as i32 * 2;
            let rgn = windows::Win32::Graphics::Gdi::CreateRoundRectRgn(0, 0, w as i32 + 1, h as i32 + 1, r, r);
            let _ = windows::Win32::Graphics::Gdi::SetWindowRgn(self.popup.hwnd, Some(rgn), true);
        }
        self.open = true;
        Ok(())
    }

    pub fn hide(&mut self) {
        if self.open {
            self.open = false;
            self.drag = None;
            self.hover = None;
            self.popup.hide();
        }
    }

    /// Move the drop-down back above the panel (the panel may have moved or resized).
    pub fn reposition(&self, monitor: RECT, dock_top: f32) {
        if !self.open {
            return;
        }
        let scale = self.scale;
        let (w, h) = ((self.size.0 * scale).ceil(), (self.size.1 * scale).ceil());
        let right = monitor.right - (12.0 * scale) as i32;
        let bottom = (dock_top - 8.0 * scale).round() as i32;
        self.popup.show(RECT { left: right - w as i32, top: bottom - h as i32, right, bottom }, false);
    }

    // ------------------------------------------------------------------ paint

    /// Repaint the drop-down; the size follows the content (the network list, the theme).
    pub fn render(&mut self, c: &Compositor, gfx: &Gfx, backdrop: &CompositionBrush, d: &Data, tray_brushes: &HashMap<u64, (u64, CompositionSurfaceBrush)>) -> Result<()> {
        self.size = (W, self.height(d));
        let (w, h) = self.size;
        let scale = self.scale;
        let (pw, ph) = ((w * scale).ceil(), (h * scale).ceil());
        if self.surface.as_ref().map(|(sw, sh, sc, _)| (*sw, *sh, *sc)) != Some((pw, ph, scale)) {
            self.surface = Some((pw, ph, scale, gfx.surface(pw, ph)?));
        }
        let Some((_, _, _, surface)) = &self.surface else { return Ok(()) };
        let (mut hits, mut icons) = (Vec::new(), Vec::new());
        let (hover, networks_open) = (self.hover, self.networks_open);
        gfx.draw(surface, |dc| {
            let cv = Canvas::new(dc, &gfx.factory, &gfx.dwrite, w, h, scale, theme::light());
            let (h_, i_) = paint(&cv, d, networks_open, hover);
            hits = h_;
            icons = i_;
            Ok(())
        })?;
        self.hits = hits;
        self.icons = icons;
        // The visual tree: the desktop behind the glass, the card, then the tray icons.
        let root = &self.popup.root;
        root.Children()?.RemoveAll()?;
        let behind = c.CreateSpriteVisual()?;
        behind.SetBrush(backdrop)?;
        behind.SetSize(ui::v2(pw, ph))?;
        root.Children()?.InsertAtTop(&behind)?;
        let sprite = ui::surface_sprite(c, surface, pw, ph)?;
        root.Children()?.InsertAtTop(&sprite)?;
        for (x, y, size, id) in &self.icons {
            let Some(key) = id.strip_prefix("tray:").and_then(|k| k.parse::<u64>().ok()) else { continue };
            if let Some((_, brush)) = tray_brushes.get(&key) {
                let sp = c.CreateSpriteVisual()?;
                let brush: CompositionBrush = brush.cast()?;
                sp.SetBrush(&brush)?;
                sp.SetSize(ui::v2(size * scale, size * scale))?;
                sp.SetOffset(ui::v3(x * scale, y * scale, 0.0))?;
                root.Children()?.InsertAtTop(&sp)?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ clicks

    /// The action under a window-local point, with the fraction across its rectangle.
    pub fn at(&self, x: f32, y: f32) -> Option<(CAct, f32, Rect)> {
        let k = self.scale;
        let (lx, ly) = (x / k, y / k);
        self.hits
            .iter()
            .rev()
            .find(|hit| inside(hit.rect, lx, ly))
            .map(|hit| (hit.act.clone(), ((lx - hit.rect.0) / hit.rect.2).clamp(0.0, 1.0), hit.rect))
    }

    /// Volume and brightness take drags; the rest are plain clicks.
    pub fn takes_drag(act: &CAct) -> bool {
        matches!(act, CAct::Do(WAct::Volume) | CAct::Do(WAct::Brightness))
    }

    pub fn start_drag(&mut self, rect: Rect, act: CAct) {
        self.drag = Some((rect, act));
    }

    /// While dragging a slider: the action and the new fraction at this point.
    pub fn drag_at(&self, x: f32, _y: f32) -> Option<(CAct, f32)> {
        let k = self.scale;
        let (rect, act) = self.drag.as_ref()?;
        let fx = ((x / k - rect.0) / rect.2).clamp(0.0, 1.0);
        Some((act.clone(), fx))
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Track the pointer for the hover highlight. Returns true when it changed.
    pub fn on_move(&mut self, x: f32, y: f32) -> bool {
        let k = self.scale;
        let (lx, ly) = (x / k, y / k);
        let found = self.hits.iter().rev().find(|hit| inside(hit.rect, lx, ly)).map(|hit| hit.rect);
        if found == self.hover {
            return false;
        }
        self.hover = found;
        true
    }

    pub fn clear_hover(&mut self) -> bool {
        if self.hover.is_none() {
            return false;
        }
        self.hover = None;
        true
    }
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

// ---------------------------------------------------------------------- painting

fn label(c: &Canvas, text: &str, x: f32, y: f32, size: f32, weight: u32, col: Col, max_w: f32) {
    c.tw(&crate::strings::tr(text), x, y, size, weight, col, max_w);
}

fn separator(c: &Canvas, y: f32) {
    c.rr(PAD, y, c.w - 2.0 * PAD, 1.0, 0.0, c.sep());
}

/// Plates inside the card. The shared 6% ink is right on a glass shelf, but on the card's
/// own fill it disappears; 10–11% still reads as a surface.
fn plate(c: &Canvas) -> Col {
    if c.light {
        (0.0, 0.0, 0.0, 0.11)
    } else {
        (1.0, 1.0, 1.0, 0.10)
    }
}

/// The card's fill: glass in both themes, but solid enough to hold the text that sits on it.
/// A fixed near-white plate was invisible text on the dark theme — its ink is light.
fn card_fill(c: &Canvas) -> Col {
    if c.light {
        (1.0, 1.0, 1.0, 0.80)
    } else {
        (0.11, 0.11, 0.13, 0.82)
    }
}

/// The edge that keeps the card's shape on any background.
fn card_edge(c: &Canvas) -> Col {
    if c.light {
        (0.0, 0.0, 0.0, 0.16)
    } else {
        (1.0, 1.0, 1.0, 0.14)
    }
}

/// Draw the whole drop-down. Returns its clickable areas and the tray icons to place on top.
fn paint(c: &Canvas, d: &Data, networks_open: bool, hover: Option<Rect>) -> (Vec<Hit>, Vec<(f32, f32, f32, String)>) {
    let mut hits: Vec<Hit> = Vec::new();
    let mut icons: Vec<(f32, f32, f32, String)> = Vec::new();
    let mut y = PAD;
    let inner = c.w - 2.0 * PAD;

    // The card itself, over the blurred desktop the window shows. It has to work twice: on a
    // white app window (where a 62% white plate with a 6% edge vanished) and on the dark
    // theme (where a fixed white plate turned its own light text invisible).
    c.rr(0.0, 0.0, c.w, c.h, RADIUS, card_fill(c));
    c.rr_stroke(0.75, 0.75, c.w - 1.5, c.h - 1.5, RADIUS - 0.5, card_edge(c), 1.5);
    // A second, fainter ring just inside: a glass rim rather than a pencil line.
    c.rr_stroke(2.0, 2.0, c.w - 4.0, c.h - 4.0, RADIUS - 2.0, c.ink(0.05), 1.0);

    // ---- header -----------------------------------------------------------
    label(c, "Control Center", PAD, y + 6.0, 15.0, 600, c.text1(), inner - 34.0);
    let gear = (c.w - PAD - 26.0, y + 2.0, 26.0, 26.0);
    c.rr(gear.0, gear.1, gear.2, gear.3, 8.0, plate(c));
    c.glyph(G_SETTINGS, gear.0 + 13.0, gear.1 + 13.0, 15.0, c.text2());
    hits.push(Hit { rect: gear, act: CAct::Do(WAct::Open("ms-settings:".into())) });
    y += HEAD_H + 10.0;

    // ---- quick tiles ------------------------------------------------------
    let tile_w = (inner - 2.0 * TILE_GAP) / 3.0;
    let dark = d.control.dark;
    let tiles: [(&str, &str, bool, Col, WAct); 6] = [
        (G_WIFI, "Wi-Fi", d.control.wifi.unwrap_or(false), BLUE, WAct::Wifi),
        (G_BT, "Bluetooth", d.control.bluetooth.unwrap_or(false), BLUE, WAct::Bluetooth),
        (G_PLANE, "Airplane Mode", d.airplane, ORANGE, WAct::Airplane),
        (G_MOON, "Night Light", d.night_light, PURPLE, WAct::NightLight),
        (G_DND, "Do Not Disturb", d.dnd, BLUE, WAct::Dnd),
        (G_SUN, if dark { "Light Mode" } else { "Dark Mode" }, dark, YELLOW, WAct::Dark),
    ];
    for (i, (glyph, text, on, col, act)) in tiles.iter().enumerate() {
        let (row, column) = (i / 3, i % 3);
        let x = PAD + column as f32 * (tile_w + TILE_GAP);
        let ty = y + row as f32 * (TILE_H + TILE_GAP);
        c.rr(x, ty, tile_w, TILE_H, 14.0, if *on { alpha(*col, 0.22) } else { plate(c) });
        if *on {
            c.rr_stroke(x + 0.5, ty + 0.5, tile_w - 1.0, TILE_H - 1.0, 14.0, alpha(*col, 0.5), 1.0);
        }
        c.rr(x + 11.0, ty + 9.0, 28.0, 28.0, 9.0, if *on { *col } else { c.ink(0.10) });
        c.glyph(glyph, x + 25.0, ty + 23.0, 15.0, if *on { WHITE } else { c.text1() });
        label(c, text, x + 11.0, ty + 42.0, 11.0, 600, if *on { c.text1() } else { c.text2() }, tile_w - 18.0);
        hits.push(Hit { rect: (x, ty, tile_w, TILE_H), act: CAct::Do(act.clone()) });
    }
    y += TILE_H * 2.0 + TILE_GAP + SEP_H;
    separator(c, y - SEP_H / 2.0);

    // ---- sliders ----------------------------------------------------------
    let volume = d.control.volume.unwrap_or(0.6);
    let muted = d.control.muted.unwrap_or(false);
    y = slider_row(c, &mut hits, y, if muted { G_MUTE } else { G_VOL }, "Sound", if muted { "Muted".to_string() } else { format!("{:.0}%", volume * 100.0) }, if muted { 0.0 } else { volume }, Some(WAct::Mute), Some(WAct::Volume), BLUE);
    // The microphone has one switch, not a level: Windows exposes mute only.
    y = slider_row(c, &mut hits, y, G_MIC, "Microphone", if d.mic_muted { "Off".to_string() } else { "On".to_string() }, if d.mic_muted { 0.0 } else { 1.0 }, Some(WAct::Mic), None, if d.mic_muted { ORANGE } else { GREEN });
    let bright = d.brightness.unwrap_or(0.7);
    let (bright_text, bright_frac) = if d.brightness_unsupported {
        ("Not supported".to_string(), 0.0)
    } else {
        (format!("{:.0}%", bright * 100.0), bright)
    };
    y = slider_row(c, &mut hits, y, G_SUN, "Brightness", bright_text, bright_frac, None, Some(WAct::Brightness), YELLOW);
    y += SEP_H;
    separator(c, y - SEP_H / 2.0);

    // ---- network ----------------------------------------------------------
    let networks: Vec<&laminar_sys::radios::WifiNetwork> = match d.wifi {
        Some(Ok(list)) => list.iter().collect(),
        _ => Vec::new(),
    };
    let current = networks.iter().find(|n| n.connected).map(|n| n.ssid.clone());
    let head = (PAD - 6.0, y, inner + 12.0, 30.0);
    hits.push(Hit { rect: head, act: CAct::Networks });
    c.glyph(G_WIFI, PAD + 10.0, y + 15.0, 16.0, if d.airplane { c.text3() } else { c.text2() });
    label(c, "Network", PAD + 26.0, y + 7.0, 12.0, 600, c.text2(), 64.0);
    let sub = match (d.airplane, &current) {
        (true, _) => "Airplane mode".to_string(),
        (false, Some(ssid)) => ssid.clone(),
        _ if networks.is_empty() => "No networks".to_string(),
        _ => format!("{} available", networks.len()),
    };
    c.tw(&sub, PAD + 92.0, y + 6.0, 12.5, 500, c.text2(), inner - 92.0 - 26.0);
    c.glyph(if networks_open { G_CHEV_UP } else { G_CHEV }, c.w - PAD - 12.0, y + 15.0, 12.0, c.text3());
    y += 30.0;

    if networks_open {
        if networks.is_empty() {
            let message = match d.wifi {
                Some(Err(code)) if *code == 5 => "Turn on Location access to see networks",
                Some(Err(_)) | None => "Looking for networks…",
                _ => "No networks in range",
            };
            label(c, message, PAD + 10.0, y + 8.0, 12.0, 400, c.text3(), inner - 20.0);
            y += NET_ROW_H;
        } else {
            for net in networks.iter().take(NET_SHOWN) {
                let row = (PAD - 6.0, y, inner + 12.0, NET_ROW_H - 4.0);
                if net.connected {
                    c.rr(row.0, row.1, row.2, row.3, 9.0, alpha(BLUE, 0.14));
                }
                let strength = (net.signal_percent as f32 / 100.0).clamp(0.1, 1.0);
                for b in 0..4 {
                    let bh = 4.0 + b as f32 * 3.0;
                    let on = strength > (b as f32 + 1.0) / 4.5;
                    let bx = c.w - PAD - 34.0 + b as f32 * 6.0;
                    c.rr(bx, y + 20.0 - bh, 3.0, bh, 1.5, if on { c.text2() } else { c.track() });
                }
                if net.secured {
                    c.glyph(G_LOCK, c.w - PAD - 48.0, y + 11.0, 11.0, c.text3());
                }
                label(c, &net.ssid, PAD + 10.0, y + 6.0, 12.5, if net.connected { 600 } else { 500 }, c.text1(), inner - 92.0);
                if !net.connected {
                    hits.push(Hit { rect: row, act: CAct::Do(WAct::JoinWifi(net.ssid.clone())) });
                }
                y += NET_ROW_H;
            }
        }
    }
    y += SEP_H;
    separator(c, y - SEP_H / 2.0);

    // ---- now playing ------------------------------------------------------
    let art = (PAD, y + 6.0, 44.0, 44.0);
    c.rr(art.0, art.1, art.2, art.3, 11.0, plate(c));
    c.glyph(G_MUSIC, art.0 + 22.0, art.1 + 22.0, 20.0, c.text3());
    match d.control.media.as_ref() {
        Some(np) => {
            let right = 3.0 * 30.0 + PAD;
            label(c, &np.title, PAD + 54.0, y + 8.0, 12.5, 600, c.text1(), c.w - PAD - 54.0 - right);
            label(c, &np.artist, PAD + 54.0, y + 27.0, 11.5, 400, c.text2(), c.w - PAD - 54.0 - right);
            let mut bx = c.w - PAD - 3.0 * 30.0;
            for (glyph, act) in [(G_PREV, WAct::MediaPrev), (if np.playing { G_PAUSE } else { G_PLAY }, WAct::MediaPlay), (G_NEXT, WAct::MediaNext)] {
                c.circle(bx + 13.0, y + 28.0, 13.0, plate(c));
                c.glyph(glyph, bx + 13.0, y + 28.0, 13.0, c.text1());
                hits.push(Hit { rect: (bx, y + 15.0, 26.0, 26.0), act: CAct::Do(act) });
                bx += 30.0;
            }
        }
        None => label(c, "Nothing playing", PAD + 54.0, y + 16.0, 12.5, 500, c.text3(), c.w - PAD - 54.0 - PAD),
    }
    y += PLAYER_H + SEP_H;
    separator(c, y - SEP_H / 2.0);

    // ---- footer: battery, then the tray icons -----------------------------
    match &d.battery {
        Some((percent, charging, left)) => {
            c.glyph(if *charging { G_PLUG } else { G_BATTERY }, PAD + 8.0, y + 15.0, 15.0, if *charging { GREEN } else { c.text2() });
            let text = if left.is_empty() { format!("{percent:.0}%") } else { format!("{percent:.0}% · {left}") };
            label(c, &text, PAD + 24.0, y + 7.0, 12.0, 500, c.text2(), 130.0);
        }
        None => {
            c.glyph(G_PLUG, PAD + 8.0, y + 15.0, 15.0, c.text3());
            label(c, "Plugged in", PAD + 24.0, y + 7.0, 12.0, 500, c.text3(), 130.0);
        }
    }
    let mut tx = c.w - PAD - 22.0;
    for (key, _tip) in d.tray.iter().rev() {
        if tx < PAD + 170.0 {
            break;
        }
        icons.push((tx, y + 5.0, 22.0, format!("tray:{key}")));
        hits.push(Hit { rect: (tx, y + 5.0, 22.0, 22.0), act: CAct::Do(WAct::Tray(*key)) });
        tx -= 26.0;
    }

    // A soft highlight on whatever the pointer is over.
    if let Some(r) = hover {
        c.rr(r.0, r.1, r.2, r.3, 12.0, c.ink(0.06));
    }
    (hits, icons)
}

/// One slider row: glyph (with an optional toggle), name, bar, value. Returns the next y.
#[allow(clippy::too_many_arguments)]
fn slider_row(c: &Canvas, hits: &mut Vec<Hit>, y: f32, glyph: &str, text: &str, value_text: String, frac: f32, button: Option<WAct>, slider: Option<WAct>, col: Col) -> f32 {
    let pad = PAD;
    if let Some(act) = &button {
        c.circle(pad + 14.0, y + 23.0, 15.0, plate(c));
        c.glyph(glyph, pad + 14.0, y + 23.0, 15.0, c.text1());
        hits.push(Hit { rect: (pad - 1.0, y + 8.0, 30.0, 30.0), act: CAct::Do(act.clone()) });
    } else {
        c.glyph(glyph, pad + 14.0, y + 23.0, 16.0, c.text2());
    }
    label(c, text, pad + 38.0, y + 4.0, 12.0, 600, c.text2(), 120.0);
    let track = (pad + 38.0, y + 24.0, c.w - 2.0 * pad - 38.0 - 44.0, 6.0);
    c.rr(track.0, track.1, track.2, track.3, 3.0, c.track());
    let frac = frac.clamp(0.0, 1.0);
    let fill = (track.2 * frac).max(if frac > 0.0 { 6.0 } else { 0.0 });
    if fill > 0.0 {
        c.rr(track.0, track.1, fill, track.3, 3.0, col);
        c.circle(track.0 + fill, track.1 + 3.0, 6.0, WHITE);
    }
    c.tr(&value_text, c.w - pad, y + 15.0, 12.0, 600, c.text1());
    if let Some(act) = slider {
        hits.push(Hit { rect: (track.0, y + 12.0, track.2, 18.0), act: CAct::Do(act) });
    }
    y + ROW_H
}
