//! Custom drawing primitives: braille area graphs, gradient bars, sparklines
//! and the spinning fan.

use std::f64::consts::TAU;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::Span;
use ratatui::widgets::canvas::{Canvas, Circle, Context, Line as CLine};
use ratatui::widgets::Widget;

use crate::theme::{gradient, mix, Theme};

/// Braille dot bit for sub-column `x` (0..2) and sub-row `y` (0..4, top first).
const BRAILLE: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// A filled braille area graph. Each value (0..=1, NaN = no data) covers one
/// braille dot column, so `values.len()` should be `area.width * 2`.
/// With `down` set, the graph mirrors around the horizontal center line: `up`
/// grows upwards and `down` grows downwards (used for rx/tx, read/write).
pub struct AreaGraph<'a> {
    pub up: &'a [f64],
    pub up_colors: &'a [Color],
    pub down: Option<(&'a [f64], &'a [Color])>,
    pub grid: Option<Color>,
}

impl Widget for AreaGraph<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let rows = area.height as usize * 4;
        let mid = if self.down.is_some() { rows / 2 } else { rows };
        let fill = |vals: &[f64], i: usize, span: usize| -> usize {
            match vals.get(i) {
                Some(v) if v.is_finite() && *v > 0.0 => ((v.min(1.0) * span as f64).round() as usize).max(1),
                _ => 0,
            }
        };

        for cy in 0..area.height as usize {
            for cx in 0..area.width as usize {
                let mut mask = 0u8;
                for (sx, dots) in BRAILLE.iter().enumerate() {
                    let col = cx * 2 + sx;
                    let up_fill = fill(self.up, col, mid);
                    let down_fill = self.down.map(|(d, _)| fill(d, col, rows - mid)).unwrap_or(0);
                    for (sy, bits) in dots.iter().enumerate() {
                        let dy = cy * 4 + sy;
                        let on = if dy < mid { dy >= mid - up_fill } else { dy < mid + down_fill };
                        if on {
                            mask |= bits;
                        }
                    }
                }
                let cell_mid_dy = cy * 4 + 2;
                let (stops, t) = match self.down {
                    Some((_, c)) if cell_mid_dy >= mid => (c, (cell_mid_dy - mid) as f64 / (rows - mid) as f64),
                    _ => (self.up_colors, (mid as f64 - cell_mid_dy as f64) / mid as f64),
                };
                let x = area.x + cx as u16;
                let y = area.y + cy as u16;
                if mask != 0 {
                    let ch = char::from_u32(0x2800 + mask as u32).unwrap_or(' ');
                    buf[(x, y)].set_char(ch).set_fg(gradient(stops, t));
                } else if let Some(g) = self.grid {
                    let is_grid_row = if self.down.is_some() {
                        cy * 4 + 4 > mid && cy * 4 <= mid
                    } else {
                        cy + 1 == area.height as usize || (area.height >= 6 && cy == area.height as usize / 2)
                    };
                    if is_grid_row {
                        buf[(x, y)].set_char('┈').set_fg(g);
                    }
                }
            }
        }
    }
}

/// A horizontal bar that uses 1/8th-cell blocks for smooth edges and colors
/// each cell along a gradient.
pub fn gradient_bar(buf: &mut Buffer, area: Rect, ratio: f64, stops: &[Color], track: Color) {
    const PARTIAL: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let w = area.width as usize;
    if w == 0 || area.height == 0 {
        return;
    }
    let eighths = (ratio.clamp(0.0, 1.0) * w as f64 * 8.0).round() as usize;
    for i in 0..w {
        let cell = &mut buf[(area.x + i as u16, area.y)];
        let filled = eighths.saturating_sub(i * 8).min(8);
        let color = gradient(stops, (i as f64 + 0.5) / w as f64);
        match filled {
            8 => {
                cell.set_char('█').set_fg(color);
            }
            0 => {
                cell.set_char('━').set_fg(track);
            }
            n => {
                cell.set_char(PARTIAL[n]).set_fg(color);
            }
        }
    }
}

/// Single-row block sparkline (▁▂▃▄▅▆▇█) colored by value.
pub fn sparkline<'a>(values: &[f64], max: f64, color: impl Fn(f64) -> Color) -> Vec<Span<'a>> {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    values
        .iter()
        .map(|v| {
            if !v.is_finite() {
                return Span::raw(" ");
            }
            let t = (v / max).clamp(0.0, 1.0);
            let ch = LEVELS[((t * 7.0).round() as usize).min(7)];
            Span::styled(ch.to_string(), Style::default().fg(color(*v)))
        })
        .collect()
}

pub struct FanArt<'a> {
    pub theme: &'a Theme,
    pub angle: f64,
    /// Angular speed in rad/s, used for motion blur.
    pub speed: f64,
    /// 0..=1 speed between the fan's min and max RPM; drives the glow color.
    pub intensity: f64,
}

impl FanArt<'_> {
    const BLADES: usize = 7;

    #[allow(clippy::too_many_arguments)]
    fn blade(&self, ctx: &mut Context, cx: f64, cy: f64, r_hub: f64, r_tip: f64, angle: f64, colors: (Color, Color)) {
        // Each blade is a curved, swept wedge: its centerline twists as it moves
        // outward and it widens toward the tip. We fill it with short chords at
        // many radii.
        let sweep = 0.75;
        let mut r = r_hub;
        while r <= r_tip {
            let t = (r - r_hub) / (r_tip - r_hub);
            let center = angle + sweep * t;
            let half_width = 0.09 + 0.15 * t;
            let color = mix(colors.0, colors.1, t);
            let (a0, a1) = (center - half_width, center + half_width);
            let steps = ((a1 - a0) * r / 1.2).ceil().max(1.0) as usize;
            for s in 0..steps {
                let p0 = a0 + (a1 - a0) * s as f64 / steps as f64;
                let p1 = a0 + (a1 - a0) * (s + 1) as f64 / steps as f64;
                ctx.draw(&CLine::new(cx + r * p0.cos(), cy + r * p0.sin(), cx + r * p1.cos(), cy + r * p1.sin(), color));
            }
            r += 0.5;
        }
    }
}

impl Widget for FanArt<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 2 {
            return;
        }
        // One canvas unit = one braille dot. Dots are roughly square because a
        // cell is ~1:2 and holds 2x4 dots, so circles come out round.
        let (w, h) = (area.width as f64 * 2.0, area.height as f64 * 4.0);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let radius = (w.min(h) / 2.0 - 0.5).max(2.0);
        let r_shroud = radius;
        let r_tip = radius - 2.0;
        let r_hub = (radius * 0.22).max(1.5);
        let th = self.theme;

        let glow = self.intensity.clamp(0.0, 1.0);
        let blade_inner = mix(th.blue, th.sapphire, glow);
        let blade_outer = mix(th.sky, th.teal, glow * 0.6);
        let ghost = mix(th.blue, th.base, 0.62);
        let blur_steps = ((self.speed / TAU) * 1.2).round().clamp(0.0, 3.0) as usize;

        Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, w])
            .y_bounds([0.0, h])
            .background_color(th.base)
            .paint(|ctx| {
                // Square housing with mounting screws, like a PC case fan.
                let s = radius + 0.5;
                for (x1, y1, x2, y2) in [
                    (cx - s, cy - s, cx + s, cy - s),
                    (cx - s, cy + s, cx + s, cy + s),
                    (cx - s, cy - s, cx - s, cy + s),
                    (cx + s, cy - s, cx + s, cy + s),
                ] {
                    ctx.draw(&CLine::new(x1, y1, x2, y2, th.surface1));
                }
                for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    let off = s - 1.6;
                    ctx.draw(&Circle { x: cx + dx * off, y: cy + dy * off, radius: 0.8, color: th.overlay0 });
                }
                ctx.draw(&Circle { x: cx, y: cy, radius: r_shroud, color: mix(th.blue, th.base, 0.45) });
                ctx.layer();

                // Motion-blur ghosts trailing behind the blades.
                for k in (1..=blur_steps).rev() {
                    let lag = k as f64 * 0.11 * (self.speed / TAU).min(3.0);
                    let c = mix(ghost, th.base, k as f64 / (blur_steps + 1) as f64);
                    for b in 0..Self::BLADES {
                        let a = self.angle - lag + b as f64 * TAU / Self::BLADES as f64;
                        self.blade(ctx, cx, cy, r_hub, r_tip, a, (c, c));
                    }
                }
                ctx.layer();

                for b in 0..Self::BLADES {
                    let a = self.angle + b as f64 * TAU / Self::BLADES as f64;
                    self.blade(ctx, cx, cy, r_hub, r_tip, a, (blade_inner, blade_outer));
                }
                ctx.layer();

                // Hub with a rotating highlight.
                let mut r = r_hub;
                while r > 0.0 {
                    ctx.draw(&Circle { x: cx, y: cy, radius: r, color: th.surface2 });
                    r -= 0.6;
                }
                ctx.draw(&Circle { x: cx, y: cy, radius: r_hub, color: th.lavender });
                let ha = -self.angle * 0.5;
                ctx.draw(&CLine::new(
                    cx,
                    cy,
                    cx + (r_hub - 0.6) * ha.cos(),
                    cy + (r_hub - 0.6) * ha.sin(),
                    th.sky,
                ));
            })
            .render(area, buf);
    }
}
