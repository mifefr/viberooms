//! Minimal anti-aliased RGBA rendering of gauges: menu bar icon and menu item bars.

pub type Rgb = (u8, u8, u8);

pub const GOOD: Rgb = (0x0c, 0xa3, 0x0c);
pub const WARNING: Rgb = (0xfa, 0xb2, 0x19);
pub const CRITICAL: Rgb = (0xd0, 0x3b, 0x3b);
pub const MUTED: Rgb = (0x89, 0x87, 0x81);

pub fn level_color(used: f64) -> Rgb {
    if used >= 85.0 {
        CRITICAL
    } else if used >= 60.0 {
        WARNING
    } else {
        GOOD
    }
}

pub fn hex(s: &str) -> Option<Rgb> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some((p(0)?, p(2)?, p(4)?))
}

/// Account identity colours (same slots as the web page, light variant).
pub fn account_color(slot: &str) -> Rgb {
    match slot {
        "acc-1" => (0x2a, 0x78, 0xd6),
        "acc-2" => (0xeb, 0x68, 0x34),
        "acc-3" => (0x1b, 0xaf, 0x7a),
        "acc-4" => (0x62, 0x50, 0xd6),
        "acc-5" => (0xe8, 0x7b, 0xa4),
        other => hex(other).unwrap_or(MUTED),
    }
}

pub struct Canvas {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h, px: vec![0; (w * h * 4) as usize] }
    }

    fn blend(&mut self, x: u32, y: u32, c: Rgb, a: f64) {
        if x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        let sa = a.clamp(0.0, 1.0);
        let da = self.px[i + 3] as f64 / 255.0;
        let oa = sa + da * (1.0 - sa);
        if oa <= 0.0 {
            return;
        }
        let mix = |s: u8, d: u8| ((s as f64 * sa + d as f64 * da * (1.0 - sa)) / oa).round() as u8;
        self.px[i] = mix(c.0, self.px[i]);
        self.px[i + 1] = mix(c.1, self.px[i + 1]);
        self.px[i + 2] = mix(c.2, self.px[i + 2]);
        self.px[i + 3] = (oa * 255.0).round() as u8;
    }

    /// Rounded rectangle, coverage from a signed distance (smooth edges).
    #[allow(clippy::too_many_arguments)] // drawing primitive: coordinates + style
    pub fn rrect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, r: f64, c: Rgb, alpha: f64) {
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let r = r.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
        for y in (y0.floor().max(0.0) as u32)..(y1.ceil() as u32).min(self.h) {
            for x in (x0.floor().max(0.0) as u32)..(x1.ceil() as u32).min(self.w) {
                let (px, py) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                let qx = px.abs() - hx;
                let qy = py.abs() - hy;
                let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
                let d = outside + qx.max(qy).min(0.0) - r;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                self.blend(x, y, c, cov * alpha);
            }
        }
    }
}

/// Menu bar icon: one column per account, filled by headroom (like a battery). @2x render: 36 px (18 pt).
pub fn tray_icon(gauges: &[(f64, f64, bool)]) -> Canvas {
    // (headroom 0-100, max utilisation 0-100, in error)
    let n = gauges.len().max(1) as f64;
    let (bw, gap, pad) = (9.0, 5.0, 2.0);
    let w = (pad * 2.0 + n * bw + (n - 1.0) * gap).ceil() as u32;
    let h = 36u32;
    let mut c = Canvas::new(w.max(16), h);
    let (top, bottom) = (5.0, 31.0);
    if gauges.is_empty() {
        c.rrect(pad, top, pad + bw, bottom, 3.0, MUTED, 0.55);
        return c;
    }
    for (i, &(headroom, used, error)) in gauges.iter().enumerate() {
        let x0 = pad + i as f64 * (bw + gap);
        c.rrect(x0, top, x0 + bw, bottom, 3.0, MUTED, 0.38);
        if error {
            c.rrect(x0, bottom - 7.0, x0 + bw, bottom, 3.0, MUTED, 1.0);
            continue;
        }
        if headroom <= 0.5 {
            // Blocked account: whole column tinted red.
            c.rrect(x0, top, x0 + bw, bottom, 3.0, CRITICAL, 0.85);
            continue;
        }
        let fill = ((headroom / 100.0).clamp(0.0, 1.0) * (bottom - top)).max(4.0);
        c.rrect(x0, bottom - fill, x0 + bw, bottom, 3.0, level_color(used), 1.0);
    }
    c
}

/// Horizontal bar for a menu item (@2x render, 18 pt high, thin centred bar).
pub fn menu_bar(used: f64, color: Rgb) -> Canvas {
    let (w, h) = (88u32, 36u32);
    let mut c = Canvas::new(w, h);
    let (x0, x1, y0, y1) = (2.0, w as f64 - 2.0, 13.0, 23.0);
    c.rrect(x0, y0, x1, y1, 5.0, MUTED, 0.30);
    let fw = ((used / 100.0).clamp(0.0, 1.0) * (x1 - x0)).max(if used > 0.0 { 6.0 } else { 0.0 });
    c.rrect(x0, y0, x0 + fw, y1, 5.0, color, 1.0);
    c
}

/// Account identity swatch for section headers.
pub fn swatch(color: Rgb) -> Canvas {
    let mut c = Canvas::new(36, 36);
    c.rrect(9.0, 9.0, 27.0, 27.0, 5.0, color, 1.0);
    c
}
