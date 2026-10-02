//! A subtle twinkling starfield painted into empty cells after the UI renders,
//! with the occasional shooting star.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::theme::{mix, Theme};

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_4D39);
    h ^ (h >> 15)
}

fn unit(h: u32) -> f64 {
    (h & 0xFFFF) as f64 / 65535.0
}

pub fn render(buf: &mut Buffer, area: Rect, theme: &Theme, t: f64) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let h = hash(x as u32, y as u32, 7);
            if h % 1000 >= 14 {
                continue;
            }
            let cell = &mut buf[(x, y)];
            if cell.symbol() != " " {
                continue;
            }
            let phase = unit(h >> 8) * std::f64::consts::TAU;
            let speed = 0.4 + unit(h >> 16) * 1.4;
            let twinkle = ((t * speed + phase).sin() * 0.5 + 0.5).powf(2.2);
            let kind = h % 97;
            let (glyph, tint) = match kind {
                0..=2 => ("✦", theme.lavender),
                3..=9 => ("∙", theme.sky),
                10..=30 => ("·", theme.overlay1),
                _ => (".", theme.overlay0),
            };
            let strength = 0.08 + twinkle * if kind <= 9 { 0.55 } else { 0.42 };
            cell.set_symbol(glyph).set_fg(mix(theme.base, tint, strength));
        }
    }

    shooting_star(buf, area, theme, t);
}

/// Every ~23s a meteor streaks diagonally across a random spot.
fn shooting_star(buf: &mut Buffer, area: Rect, theme: &Theme, t: f64) {
    const PERIOD: f64 = 23.0;
    const DURATION: f64 = 1.3;
    let cycle = (t / PERIOD).floor() as u32;
    let local = t - cycle as f64 * PERIOD;
    if local > DURATION || area.width < 20 || area.height < 6 {
        return;
    }
    let h = hash(cycle, 3, 99);
    let start_x = area.x as f64 + unit(h) * area.width as f64 * 0.7;
    let start_y = area.y as f64 + unit(h >> 16) * area.height as f64 * 0.4;
    let progress = local / DURATION;
    let len = 10.0;
    let travel = area.width as f64 * 0.35;
    let head_x = start_x + progress * travel;
    let head_y = start_y + progress * travel * 0.25;
    for i in 0..len as usize {
        let x = head_x - i as f64;
        let y = head_y - i as f64 * 0.25;
        if x < area.left() as f64 || y < area.top() as f64 || x >= area.right() as f64 || y >= area.bottom() as f64 {
            continue;
        }
        let cell = &mut buf[(x as u16, y as u16)];
        if cell.symbol() != " " && !matches!(cell.symbol(), "." | "·" | "∙" | "✦") {
            continue;
        }
        let fade = (1.0 - i as f64 / len) * (1.0 - progress).sqrt();
        let glyph = if i == 0 { "✧" } else { "⠂" };
        cell.set_symbol(glyph).set_fg(mix(theme.base, theme.text, fade * 0.9));
    }
}
