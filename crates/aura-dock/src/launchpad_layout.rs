//! Screen-local geometry for the compact launchpad (all values are physical pixels).

pub fn panel(w: f32, h: f32, reserve: f32, reserve_top: bool, scale: f32) -> (f32, f32, f32, f32) {
    let margin = (16.0 * scale).min(w.max(0.0) / 4.0).min(h.max(0.0) / 4.0);
    let free_w = (w - 2.0 * margin).max(1.0);
    let reserved = reserve.max(0.0).min((h - 2.0 * margin - 1.0).max(0.0));
    let free_h = (h - reserved - 2.0 * margin).max(1.0);
    let width = (620.0 * scale).min(free_w);
    let height = (600.0 * scale).min(free_h);
    let x = (w - width) / 2.0;
    // Bottom dock: align just above it. Top dock: align just below it.
    let y = margin + if reserve_top { reserved } else { free_h - height };
    (x, y, width, height)
}

#[cfg(test)]
mod tests {
    use super::panel;
    #[test]
    fn compact_and_within_screen_at_various_dpi_and_dock_positions() {
        for (w, h, reserve, s) in [(1920.0, 1080.0, 100.0, 1.0), (1920.0, 1080.0, 150.0, 1.5), (800.0, 600.0, 96.0, 1.0), (1024.0, 768.0, 130.0, 1.5)] {
            for top in [true, false] {
                let (x, y, pw, ph) = panel(w, h, reserve, top, s);
                assert!(pw <= 620.0*s && ph <= 600.0*s);
                assert!(x >= 0.0 && x+pw <= w && y >= 0.0 && y+ph <= h);
                if top { assert!(y >= reserve); } else { assert!(y+ph <= h-reserve); }
                if h - reserve - ph >= 32.0*s {
                    if top { assert!((y - reserve - 16.0*s).abs() < 0.1); }
                    else { assert!((h - reserve - y - ph - 16.0*s).abs() < 0.1); }
                }
            }
        }
    }
}
