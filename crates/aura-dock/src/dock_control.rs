//! The dock's side of the control centre: opening and closing the drop-down, feeding it
//! the system state, and routing its clicks to the panel's own handlers.
//! (A child module of `dock`, so it can use the dock's private state.)

use super::*;
use crate::control_center::{CAct, ControlCenter, Data};

impl Dock {
    /// The tray icons' (key, tooltip) list for the drop-down's footer.
    fn cc_tray(&mut self) -> Vec<(u64, String)> {
        self.tray_views().iter().map(|t| (t.key, t.tip.clone())).collect()
    }

    /// Battery for the drop-down's footer: percent, charging, time left.
    fn cc_battery(&self) -> Option<(f32, bool, String)> {
        let feed = laminar_sys::connectors::local::battery().ok()?;
        let percent = feed.value? as f32;
        let charging = feed.extra.get("charging").map(|c| c == "true").unwrap_or(false);
        Some((percent, charging, feed.extra.get("time_left").cloned().unwrap_or_default()))
    }

    /// The shelf's inner edge in screen px — the drop-down's anchor.
    pub(super) fn cc_anchor(&self) -> f32 {
        self.shelf_inner_y() as f32
    }

    /// Repaint the drop-down when it is open (it follows the system state).
    pub(super) fn cc_render(&mut self) {
        if !self.control_center.open {
            return;
        }
        let tray = self.cc_tray();
        let battery = self.cc_battery();
        let data = Data {
            control: &self.control,
            wifi: self.wifi.as_ref(),
            tray: &tray,
            mic_muted: self.hub.mic_muted.unwrap_or(false),
            brightness: self.hub.brightness,
            brightness_unsupported: self.hub.brightness_unsupported,
            night_light: self.hub.night_light,
            airplane: self.hub.airplane,
            dnd: self.hub.dnd,
            battery,
        };
        let (c, gfx, bd) = (self.compositor.clone(), &self.gfx, self.backdrop.clone());
        if let Err(e) = self.control_center.render(&c, gfx, &bd, &data, &self.tray_brushes) {
            trace!("control centre failed to paint: {e:?}");
        }
    }

    pub(super) fn open_control_center(&mut self) {
        if self.control_center.open {
            return;
        }
        self.close_launchpad();
        self.hide_preview();
        self.stack_view.hide();
        if self.menu_open {
            self.menu.hide(false);
            self.menu_open = false;
        }
        // Ask for fresh numbers: the drop-down shows the volume, the networks, the battery.
        self.services.send(Req::Control);
        self.services.send(Req::WifiList);
        self.services.send(Req::Stats);
        let tray = self.cc_tray();
        let battery = self.cc_battery();
        let data = Data {
            control: &self.control,
            wifi: self.wifi.as_ref(),
            tray: &tray,
            mic_muted: self.hub.mic_muted.unwrap_or(false),
            brightness: self.hub.brightness,
            brightness_unsupported: self.hub.brightness_unsupported,
            night_light: self.hub.night_light,
            airplane: self.hub.airplane,
            dnd: self.hub.dnd,
            battery,
        };
        let (c, gfx, bd) = (self.compositor.clone(), &self.gfx, self.backdrop.clone());
        let anchor = self.cc_anchor();
        match self.control_center.show(&c, gfx, &bd, self.monitor, anchor, self.panel_top, self.scale, &data, &self.tray_brushes) {
            Ok(()) => trace!("control centre open at {anchor:.0}"),
            Err(e) => trace!("control centre failed: {e:?}"),
        }
    }

    pub(super) fn close_control_center(&mut self) {
        if !self.control_center.open {
            return;
        }
        self.control_center.hide();
    }

    pub(super) fn toggle_control_center(&mut self) {
        if self.control_center.open {
            self.close_control_center();
        } else {
            self.open_control_center();
        }
    }

    /// A click inside the drop-down: run the action it was drawn with.
    pub(super) fn cc_click(&mut self, x: f32, y: f32) {
        let Some((act, frac, rect)) = self.control_center.at(x, y) else { return };
        trace!("control centre action {act:?}");
        if let CAct::Do(a) = &act {
            if ControlCenter::takes_drag(&act) {
                self.control_center.start_drag(rect, act.clone());
                self.widget_act(a.clone(), frac);
                return;
            }
        }
        match act {
            CAct::Networks => {
                self.control_center.networks_open = !self.control_center.networks_open;
                if self.control_center.networks_open {
                    self.services.send(Req::WifiList);
                }
                // Its height changed: show it again so the window grows upwards from the panel.
                self.cc_reflow();
            }
            CAct::Do(a) => self.widget_act(a, frac),
        }
    }

    /// The drop-down changed height (the network list): re-show it above the panel.
    pub(super) fn cc_reflow(&mut self) {
        if !self.control_center.open {
            return;
        }
        let tray = self.cc_tray();
        let battery = self.cc_battery();
        let data = Data {
            control: &self.control,
            wifi: self.wifi.as_ref(),
            tray: &tray,
            mic_muted: self.hub.mic_muted.unwrap_or(false),
            brightness: self.hub.brightness,
            brightness_unsupported: self.hub.brightness_unsupported,
            night_light: self.hub.night_light,
            airplane: self.hub.airplane,
            dnd: self.hub.dnd,
            battery,
        };
        let (c, gfx, bd) = (self.compositor.clone(), &self.gfx, self.backdrop.clone());
        let anchor = self.cc_anchor();
        if let Err(e) = self.control_center.show(&c, gfx, &bd, self.monitor, anchor, self.panel_top, self.scale, &data, &self.tray_brushes) {
            trace!("control centre reflow failed: {e:?}");
        }
    }

    /// While a slider is being dragged: set it to the pointer's position.
    pub(super) fn cc_drag(&mut self, x: f32, y: f32) {
        if let Some((act, frac)) = self.control_center.drag_at(x, y) {
            if let CAct::Do(a) = act {
                self.widget_act(a, frac);
            }
        }
    }
}
