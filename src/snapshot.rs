//! Hidden `--dump-html <file> [WxH]` mode: samples for a few seconds, then renders
//! one frame offscreen and writes it as colored HTML. Handy for previews and
//! for checking layouts without a real terminal.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::Terminal;

use crate::app::App;
use crate::collector;

fn css(c: Color, fallback: &str) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_string(),
    }
}

/// Browsers fall back to fonts that draw braille and block elements badly, so
/// draw those glyphs as SVG to keep previews faithful to a real terminal.
fn glyph_html(sym: &str) -> String {
    const SVG: &str = "<svg viewBox='0 0 2 4' preserveAspectRatio='none' style='width:1ch;height:1.15em;vertical-align:top'>";
    let Some(c) = sym.chars().next().filter(|_| sym.chars().count() == 1) else {
        return sym.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    };
    let code = c as u32;
    match code {
        0x2801..=0x28FF => {
            let bits = code - 0x2800;
            let dots = [(0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08), (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80)];
            let mut out = SVG.to_string();
            for (x, y, b) in dots {
                if bits & b != 0 {
                    let _ = write!(out, "<circle cx='{}.5' cy='{}.5' r='0.36' fill='currentColor'/>", x, y);
                }
            }
            out + "</svg>"
        }
        0x2581..=0x2588 => {
            let h = (code - 0x2580) as f64 / 2.0;
            format!("{SVG}<rect x='0' y='{}' width='2' height='{h}' fill='currentColor'/></svg>", 4.0 - h)
        }
        0x2589..=0x258F => {
            let w = (0x2590 - code) as f64 / 4.0;
            format!("{SVG}<rect x='0' y='0' width='{w}' height='4' fill='currentColor'/></svg>")
        }
        _ => sym.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"),
    }
}

pub fn dump(app: &mut App, interval: Duration, path: &str, width: u16, height: u16, seconds: u64) -> Result<()> {
    let rx = collector::spawn(interval);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        while let Ok(s) = rx.try_recv() {
            app.ingest(s);
        }
        app.animate(1.0 / 30.0);
        std::thread::sleep(Duration::from_millis(33));
    }
    app.fps = 30.0;
    let mut term = Terminal::new(TestBackend::new(width, height))?;
    term.draw(|f| crate::ui::draw(f, app))?;
    let buf = term.backend().buffer();
    let th = app.theme();
    let (bg0, fg0) = (css(th.base, "#000"), css(th.text, "#fff"));
    let mut html = format!(
        "<!doctype html><meta charset=utf-8><style>body{{margin:0;background:{bg0}}}\
         pre{{margin:0;padding:12px;font:15px/1.15 Menlo,'Apple Braille',monospace;color:{fg0}}}\
         span{{white-space:pre;display:inline-block;width:1ch;overflow:visible}}</style><pre>"
    );
    for y in 0..height {
        for x in 0..width {
            let cell = &buf[(x, y)];
            let mut style = format!("color:{};background:{}", css(cell.fg, &fg0), css(cell.bg, &bg0));
            if cell.modifier.contains(ratatui::style::Modifier::BOLD) {
                style.push_str(";font-weight:bold");
            }
            let sym = glyph_html(cell.symbol());
            let _ = write!(html, "<span style=\"{style}\">{sym}</span>");
        }
        html.push('\n');
    }
    html.push_str("</pre>");
    std::fs::write(path, html)?;
    Ok(())
}
