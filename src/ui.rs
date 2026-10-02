use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, BorderType, Chart, Clear, Dataset, GraphType, Paragraph, Widget};
use ratatui::Frame;

use crate::app::{pct, App, Mode, WINDOWS};
use crate::collector::Snapshot;
use crate::fmt;
use crate::theme::{gradient, Theme};
use crate::widgets::{gradient_bar, sparkline, AreaGraph, FanArt};

pub fn draw(f: &mut Frame, app: &mut App) {
    let th = app.theme();
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(th.base).fg(th.text)), area);

    let [header, body, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(area);

    let Some(snap) = app.snap.clone() else {
        draw_header(f.buffer_mut(), header, app, None);
        draw_footer(f.buffer_mut(), footer, app);
        let msg = Paragraph::new(Line::from(vec![
            Span::styled("◐ ", Style::new().fg(th.mauve)),
            Span::styled("sampling sensors…", Style::new().fg(th.subtext)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(msg, Rect { y: body.y + body.height / 2, height: 1, ..body });
        return;
    };

    draw_header(f.buffer_mut(), header, app, Some(&snap));

    let [row1, row2, row3] =
        Layout::vertical([Constraint::Percentage(38), Constraint::Percentage(28), Constraint::Fill(1)]).areas(body);

    // Row 1: CPU history | per-core sparklines | fans
    let show_cores = body.width >= 110;
    let show_fans = body.width >= 80;
    // Size the fan panel so each fan gets a square canvas filling the row height.
    let fans_w = if show_fans {
        let n = snap.fans.len().clamp(1, 2) as u16;
        let art_h = row1.height.saturating_sub(5).max(4);
        (n * (art_h * 2 + 2) + 2).min(body.width * 3 / 10).max(24)
    } else {
        0
    };
    let [cpu_area, cores_area, fans_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(if show_cores { 36 } else { 0 }),
        Constraint::Length(fans_w),
    ])
    .areas(row1);
    draw_cpu(f.buffer_mut(), cpu_area, app);
    if show_cores {
        draw_cores(f.buffer_mut(), cores_area, app, &snap);
    }
    if show_fans {
        draw_fans(f.buffer_mut(), fans_area, app, &snap);
    }

    // Row 2: memory | network | disk
    let [mem_area, net_area, disk_area] = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .areas(row2);
    draw_memory(f.buffer_mut(), mem_area, app, &snap);
    draw_network(f.buffer_mut(), net_area, app, &snap);
    draw_disk(f.buffer_mut(), disk_area, app, &snap);

    // Row 3: temperatures | processes
    let [temp_area, proc_area] =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(row3);
    draw_temps(f, temp_area, app, &snap);
    draw_procs(f.buffer_mut(), proc_area, app);

    if app.stars {
        let t = app.started.elapsed().as_secs_f64();
        crate::stars::render(f.buffer_mut(), body, th, t);
    }

    draw_footer(f.buffer_mut(), footer, app);

    match &app.mode {
        Mode::Help => draw_help(f, area, th),
        Mode::ConfirmKill(pid, name) => draw_confirm(f, area, th, *pid, name),
        _ => {}
    }
}

fn panel<'a>(th: &Theme, title: &'a str, accent: Color) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.surface1))
        .title_top(Line::from(vec![
            Span::styled(" ", Style::new()),
            Span::styled(title, Style::new().fg(accent).add_modifier(Modifier::BOLD)),
            Span::styled(" ", Style::new()),
        ]))
}

/// Adds a right-aligned title only if it fits beside the panel name.
fn with_right<'a>(block: Block<'a>, area: Rect, title: &str, right: Line<'a>) -> Block<'a> {
    if title.len() as u16 + right.width() as u16 + 6 <= area.width {
        block.title_top(right.alignment(Alignment::Right))
    } else {
        block
    }
}

/// Writes relative-time ticks ("-5m ... now") into a panel's bottom border.
fn time_axis(buf: &mut Buffer, x: u16, width: u16, y: u16, window_secs: usize, th: &Theme) {
    let window = window_secs.max(1);
    let ticks: usize = match window {
        60 => 4,
        300 => 5,
        900 | 1800 => 3,
        _ => 4,
    };
    let label = |secs: usize| -> String {
        if secs == 0 {
            "now".into()
        } else if secs < 120 {
            format!("-{secs}s")
        } else {
            format!("-{}m", secs / 60)
        }
    };
    let style = Style::new().fg(th.overlay0);
    let min_gap = 7;
    let mut last_end: Option<u16> = None;
    for i in 0..=ticks {
        let secs = window * (ticks - i) / ticks;
        let text = label(secs);
        let pos = x + (width.saturating_sub(1) as usize * i / ticks) as u16;
        let tx = if i == 0 {
            pos
        } else if i == ticks {
            (x + width).saturating_sub(text.len() as u16)
        } else {
            pos.saturating_sub(text.len() as u16 / 2)
        };
        if let Some(end) = last_end {
            if tx < end + 2 || (width < min_gap * ticks as u16 && i != ticks) {
                continue;
            }
        }
        buf.set_string(tx, y, &text, style);
        last_end = Some(tx + text.len() as u16);
    }
}

fn window_selector(app: &App, th: &Theme) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (_, label)) in WINDOWS.iter().enumerate() {
        let style = if i == app.window_idx {
            Style::new().fg(th.yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(th.surface2)
        };
        spans.push(Span::styled(*label, style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans).alignment(Alignment::Right)
}

fn draw_header(buf: &mut Buffer, area: Rect, app: &App, snap: Option<&Snapshot>) {
    let th = app.theme();
    buf.set_style(area, Style::new().bg(th.mantle));
    let sep = || Span::styled("  │  ", Style::new().fg(th.surface1));
    let mut spans = vec![
        Span::styled(" ◆ supertop ", Style::new().fg(th.crust).bg(th.mauve).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.info.host.clone(), Style::new().fg(th.text).bold()),
        sep(),
        Span::styled(app.info.os.clone(), Style::new().fg(th.subtext)),
        sep(),
        Span::styled(app.info.cpu_brand.clone(), Style::new().fg(th.blue)),
        Span::styled(
            match app.info.physical_cores {
                Some(p) if p != app.info.cores => format!(" ({p}c/{}t)", app.info.cores),
                _ => format!(" ({} cores)", app.info.cores),
            },
            Style::new().fg(th.overlay1),
        ),
    ];
    if let Some(s) = snap {
        spans.push(sep());
        spans.push(Span::styled("up ", Style::new().fg(th.overlay1)));
        spans.push(Span::styled(fmt::duration(s.uptime), Style::new().fg(th.teal)));
        spans.push(sep());
        spans.push(Span::styled("load ", Style::new().fg(th.overlay1)));
        for l in s.load {
            let c = th.load_color(l / app.info.cores.max(1) as f64 * 100.0);
            spans.push(Span::styled(format!("{l:.2} "), Style::new().fg(c)));
        }
    }
    let left = Line::from(spans);
    let left_w = left.width() as u16;
    buf.set_line(area.x, area.y, &left, area.width);

    let mut right = vec![];
    if app.paused {
        right.push(Span::styled(" ⏸ PAUSED ", Style::new().fg(th.crust).bg(th.yellow).bold()));
        right.push(Span::raw("  "));
    }
    right.push(Span::styled(fmt::clock(), Style::new().fg(th.lavender).bold()));
    right.push(Span::raw(" "));
    let line = Line::from(right);
    let w = line.width() as u16;
    if area.width >= left_w + w + 2 {
        buf.set_line(area.right() - w, area.y, &line, w);
    }
}

fn draw_footer(buf: &mut Buffer, area: Rect, app: &App) {
    let th = app.theme();
    buf.set_style(area, Style::new().bg(th.mantle));
    let dim = Style::new().fg(th.overlay0);
    let sep = || Span::styled(" │ ", Style::new().fg(th.surface1));
    let mut spans = vec![
        Span::styled(" supertop", Style::new().fg(th.mauve).bold()),
        Span::styled(concat!(" v", env!("CARGO_PKG_VERSION")), dim),
        sep(),
        Span::styled(format!("{:.0} fps", app.fps), Style::new().fg(th.green)),
        sep(),
        Span::styled(th.name, Style::new().fg(th.subtext)),
        sep(),
    ];
    let keys: &[(&str, &str)] = &[
        ("q", "uit"),
        ("←→", " window"),
        ("s", "ort"),
        ("r", "ev"),
        ("/", " filter"),
        ("x", " kill"),
        ("t", "heme"),
        ("b", "g"),
        ("p", "ause"),
        ("?", " help"),
    ];
    for (k, rest) in keys {
        spans.push(Span::styled("[", dim));
        spans.push(Span::styled(*k, Style::new().fg(th.peach).bold()));
        spans.push(Span::styled("]", dim));
        spans.push(Span::styled(*rest, Style::new().fg(th.subtext)));
        spans.push(Span::raw(" "));
    }
    buf.set_line(area.x, area.y, &Line::from(spans), area.width);

    let right = match &app.status_msg {
        Some((msg, at)) if at.elapsed().as_secs() < 4 => Span::styled(format!(" {msg} "), Style::new().fg(th.yellow)),
        _ => Span::styled(
            format!(" {} samples · {} ", app.hist.samples, app.window_label()),
            Style::new().fg(th.overlay0),
        ),
    };
    let w = right.width() as u16;
    if area.width > w + 100 {
        buf.set_span(area.right() - w, area.y, &right, w);
    }
}

fn draw_cpu(buf: &mut Buffer, area: Rect, app: &App) {
    let th = app.theme();
    let cur = app.hist.cpu.last().unwrap_or(0.0);
    let block = panel(th, "CPU", th.green)
        .title_top(
            Line::from(vec![
                Span::styled(format!("{cur:5.1}%"), Style::new().fg(th.load_color(cur)).bold()),
                Span::styled(
                    format!("  avg {:.0}%  peak {:.0}% ", app.hist.cpu.avg_in(app.window()), app.hist.cpu.max_in(app.window())),
                    Style::new().fg(th.overlay1),
                ),
            ])
            .alignment(Alignment::Center),
        )
        .title_top(window_selector(app, th));
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.width < 8 || inner.height < 2 {
        return;
    }

    let gutter = 5;
    let graph = Rect { x: inner.x + gutter, width: inner.width - gutter, ..inner };
    let label_style = Style::new().fg(th.overlay0);
    buf.set_string(inner.x, inner.y, "100%", label_style);
    if inner.height >= 6 {
        buf.set_string(inner.x, inner.y + inner.height / 2, " 50%", label_style);
    }
    buf.set_string(inner.x, inner.bottom() - 1, "  0%", label_style);

    let vals: Vec<f64> =
        app.hist.cpu.resample(app.window(), graph.width as usize * 2).iter().map(|v| v / 100.0).collect();
    AreaGraph { up: &vals, up_colors: &th.cpu_gradient(), down: None, grid: Some(th.surface0) }.render(graph, buf);
    time_axis(buf, graph.x, graph.width, area.bottom() - 1, app.window_secs(), th);
}

fn draw_cores(buf: &mut Buffer, area: Rect, app: &App, snap: &Snapshot) {
    let th = app.theme();
    let freq = if snap.cpu_freq_mhz > 0 { format!("{:.2} GHz ", snap.cpu_freq_mhz as f64 / 1000.0) } else { String::new() };
    let block = panel(th, "Cores", th.teal)
        .title_top(Line::from(Span::styled(freq, Style::new().fg(th.overlay1))).alignment(Alignment::Right));
    let inner = block.inner(area).inner(Margin { horizontal: 1, vertical: 0 });
    block.render(area, buf);
    let n = snap.cpu_cores.len();
    if n == 0 || inner.height == 0 {
        return;
    }
    let cols = n.div_ceil(inner.height as usize).max(1);
    let col_w = inner.width as usize / cols;
    let rows_per_col = n.div_ceil(cols);
    for (i, usage) in snap.cpu_cores.iter().enumerate() {
        let (c, r) = (i / rows_per_col, i % rows_per_col);
        let x = inner.x + (c * col_w) as u16;
        let y = inner.y + r as u16;
        let spark_w = col_w.saturating_sub(10);
        let hist = app.hist.cores.get(i).map(|s| s.resample(app.window(), spark_w)).unwrap_or_default();
        let mut spans = vec![Span::styled(format!("{:>2} ", i + 1), Style::new().fg(th.overlay0))];
        spans.extend(sparkline(&hist, 100.0, |v| th.load_color(v)));
        spans.push(Span::styled(format!("{:>4.0}%", usage), Style::new().fg(th.load_color(*usage)).bold()));
        buf.set_line(x, y, &Line::from(spans), col_w as u16);
    }
}

fn draw_fans(buf: &mut Buffer, area: Rect, app: &App, snap: &Snapshot) {
    let th = app.theme();
    let total: f64 = snap.fans.iter().map(|f| f.rpm).sum();
    let mut block = panel(th, "Fans", th.sapphire);
    if !snap.fans.is_empty() && area.width >= 24 {
        block = block.title_top(
            Line::from(Span::styled(
                format!(" {} rpm ", fmt::thousands((total / snap.fans.len() as f64) as u64)),
                Style::new().fg(th.overlay1),
            ))
            .alignment(Alignment::Right),
        );
    }
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height < 4 || inner.width < 8 {
        return;
    }

    if snap.fans.is_empty() {
        let art_h = (inner.height - 2).min(inner.width / 2);
        let art = Rect { x: inner.x + (inner.width - art_h * 2) / 2, y: inner.y, width: art_h * 2, height: art_h };
        FanArt { theme: th, angle: 0.3, speed: 0.0, intensity: 0.0 }.render(art, buf);
        let msg = Line::from(Span::styled("no fans · passive cooling", Style::new().fg(th.overlay0)))
            .alignment(Alignment::Center);
        buf.set_line(inner.x, inner.y + art_h + 1, &msg, inner.width);
        return;
    }

    let n = snap.fans.len();
    let (cols, art_h) = fan_grid(n, inner.width, inner.height);
    let rows = n.div_ceil(cols);
    let cell_w = inner.width / cols as u16;
    let cell_h = inner.height / rows as u16;
    for (i, fan) in snap.fans.iter().enumerate() {
        let cell = Rect {
            x: inner.x + (i % cols) as u16 * cell_w,
            y: inner.y + (i / cols) as u16 * cell_h,
            width: cell_w,
            height: cell_h,
        };
        let art_w = art_h * 2;
        let top = cell.y + cell.height.saturating_sub(art_h + 2) / 2;
        let art = Rect { x: cell.x + (cell.width - art_w) / 2, y: top, width: art_w, height: art_h };
        FanArt {
            theme: th,
            angle: app.fan_angles.get(i).copied().unwrap_or(0.0),
            speed: app.fan_speeds.get(i).copied().unwrap_or(0.0),
            intensity: fan.ratio(),
        }
        .render(art, buf);

        let label = Line::from(vec![
            Span::styled(format!("{} ", fan.label), Style::new().fg(th.subtext)),
            if fan.rpm < 1.0 {
                Span::styled("idle", Style::new().fg(th.overlay0))
            } else {
                Span::styled(format!("{} rpm", fmt::thousands(fan.rpm as u64)), Style::new().fg(th.sky).bold())
            },
        ]);
        // Drop the fan name before truncating the speed.
        let label = if label.width() as u16 > cell.width { Line::from(label.spans[1].clone()) } else { label };
        let label_w = label.width() as u16;
        let lx = cell.x + cell.width.saturating_sub(label_w) / 2;
        buf.set_line(lx, art.bottom(), &label, cell.width);
        let bar = Rect { x: art.x + 1, y: art.bottom() + 1, width: art_w.saturating_sub(2), height: 1 };
        gradient_bar(buf, bar, fan.ratio(), &[th.blue, th.sapphire, th.sky], th.surface0);
    }
}

/// Chooses columns for `n` fans so each square fan canvas is as large as
/// possible. Returns (columns, fan height in cells); width is twice the height.
fn fan_grid(n: usize, w: u16, h: u16) -> (usize, u16) {
    (1..=n.max(1))
        .map(|cols| {
            let rows = n.div_ceil(cols) as u16;
            let cell_w = w / cols as u16;
            let cell_h = h / rows;
            (cols, cell_h.saturating_sub(2).min(cell_w.saturating_sub(2) / 2).max(1))
        })
        .max_by_key(|&(cols, art)| (art, std::cmp::Reverse(cols)))
        .unwrap_or((1, 1))
}

fn draw_memory(buf: &mut Buffer, area: Rect, app: &App, snap: &Snapshot) {
    let th = app.theme();
    let p = pct(snap.mem_used, snap.mem_total);
    let block = with_right(
        panel(th, "Memory", th.mauve),
        area,
        "Memory",
        Line::from(vec![
            Span::styled(format!("{p:.0}% "), Style::new().fg(gradient(&th.mem_gradient(), p / 100.0)).bold()),
            Span::styled(
                format!("{} / {} ", fmt::bytes(snap.mem_used as f64), fmt::bytes(snap.mem_total as f64)),
                Style::new().fg(th.overlay1),
            ),
        ]),
    );
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height < 3 {
        return;
    }
    let bars = if snap.swap_total > 0 { 2 } else { 1 };
    let graph = Rect { height: inner.height.saturating_sub(bars), ..inner };
    let vals: Vec<f64> =
        app.hist.mem.resample(app.window(), graph.width as usize * 2).iter().map(|v| v / 100.0).collect();
    AreaGraph { up: &vals, up_colors: &th.mem_gradient(), down: None, grid: Some(th.surface0) }.render(graph, buf);
    time_axis(buf, inner.x, inner.width, area.bottom() - 1, app.window_secs(), th);

    let mut y = graph.bottom();
    gauge_row(buf, inner.x, y, inner.width, "RAM ", p, &th.mem_gradient(), th, format!("{p:>3.0}%"));
    y += 1;
    if snap.swap_total > 0 {
        let sp = pct(snap.swap_used, snap.swap_total);
        gauge_row(
            buf,
            inner.x,
            y,
            inner.width,
            "Swap",
            sp,
            &[th.peach, th.maroon, th.red],
            th,
            fmt::bytes(snap.swap_used as f64),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn gauge_row(buf: &mut Buffer, x: u16, y: u16, w: u16, label: &str, p: f64, stops: &[Color], th: &Theme, value: String) {
    buf.set_string(x + 1, y, label, Style::new().fg(th.subtext));
    let vw = value.chars().count() as u16;
    let bar_x = x + 1 + label.len() as u16 + 1;
    let bar_w = w.saturating_sub(label.len() as u16 + vw + 4);
    gradient_bar(buf, Rect { x: bar_x, y, width: bar_w, height: 1 }, p / 100.0, stops, th.surface0);
    buf.set_string(bar_x + bar_w + 1, y, &value, Style::new().fg(th.text));
}

/// Shared renderer for the mirrored rx/tx and read/write panels.
#[allow(clippy::too_many_arguments)]
fn draw_mirrored(
    buf: &mut Buffer,
    area: Rect,
    app: &App,
    block: Block,
    up: &crate::history::Series,
    down: &crate::history::Series,
    up_stops: &[Color],
    down_stops: &[Color],
    labels: (&str, &str),
    footer_lines: u16,
) -> Rect {
    let th = app.theme();
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height < 3 {
        return Rect::default();
    }
    let graph = Rect { height: inner.height.saturating_sub(footer_lines), ..inner };
    let cols = graph.width as usize * 2;
    let w = app.window();
    // Each direction autoscales on its own so a quiet upload is still visible
    // next to a busy download.
    let up_max = fmt::nice_ceil(up.max_in(w).max(1024.0));
    let down_max = fmt::nice_ceil(down.max_in(w).max(1024.0));
    let u: Vec<f64> = up.resample(w, cols).iter().map(|v| v / up_max).collect();
    let d: Vec<f64> = down.resample(w, cols).iter().map(|v| v / down_max).collect();
    AreaGraph { up: &u, up_colors: up_stops, down: Some((&d, down_stops)), grid: Some(th.surface1) }.render(graph, buf);

    let dim = Style::new().fg(th.overlay0);
    buf.set_string(graph.x + 1, graph.y, format!("{} {}", labels.0, fmt::rate(up_max)), dim);
    buf.set_string(graph.x + 1, graph.bottom() - 1, format!("{} {}", labels.1, fmt::rate(down_max)), dim);
    time_axis(buf, inner.x, inner.width, area.bottom() - 1, app.window_secs(), th);
    Rect { y: graph.bottom(), height: footer_lines, ..inner }
}

fn draw_network(buf: &mut Buffer, area: Rect, app: &App, snap: &Snapshot) {
    let th = app.theme();
    let block = with_right(
        panel(th, "Network", th.sky),
        area,
        "Network",
        Line::from(vec![
            Span::styled("▼ ", Style::new().fg(th.sky)),
            Span::styled(fmt::rate(snap.net_rx_bps), Style::new().fg(th.sky).bold()),
            Span::styled("  ▲ ", Style::new().fg(th.pink)),
            Span::styled(fmt::rate(snap.net_tx_bps), Style::new().fg(th.pink).bold()),
            Span::raw(" "),
        ]),
    );
    let foot = draw_mirrored(
        buf,
        area,
        app,
        block,
        &app.hist.net_rx,
        &app.hist.net_tx,
        &th.net_down_gradient(),
        &th.net_up_gradient(),
        ("▼", "▲"),
        1,
    );
    if foot.height == 0 {
        return;
    }
    let line = Line::from(vec![
        Span::styled(format!(" {} ", snap.net_iface), Style::new().fg(th.lavender)),
        Span::styled("total ", Style::new().fg(th.overlay0)),
        Span::styled(format!("▼{} ", fmt::bytes(snap.net_rx_total as f64)), Style::new().fg(th.sky)),
        Span::styled(format!("▲{}", fmt::bytes(snap.net_tx_total as f64)), Style::new().fg(th.pink)),
    ]);
    buf.set_line(foot.x, foot.y, &line, foot.width);
}

fn draw_disk(buf: &mut Buffer, area: Rect, app: &App, snap: &Snapshot) {
    let th = app.theme();
    let block = with_right(
        panel(th, "Disk", th.peach),
        area,
        "Disk",
        Line::from(vec![
            Span::styled("R ", Style::new().fg(th.green)),
            Span::styled(fmt::rate(snap.disk_read_bps), Style::new().fg(th.green).bold()),
            Span::styled("  W ", Style::new().fg(th.peach)),
            Span::styled(fmt::rate(snap.disk_write_bps), Style::new().fg(th.peach).bold()),
            Span::raw(" "),
        ]),
    );
    let mounts = snap.mounts.len().min(if area.height >= 12 { 3 } else { 1 }) as u16;
    let foot = draw_mirrored(
        buf,
        area,
        app,
        block,
        &app.hist.disk_r,
        &app.hist.disk_w,
        &th.disk_read_gradient(),
        &th.disk_write_gradient(),
        ("R", "W"),
        mounts,
    );
    for (i, m) in snap.mounts.iter().take(mounts as usize).enumerate() {
        let p = pct(m.used, m.total);
        let name: String = if m.mount == "/" { m.name.clone() } else { m.mount.clone() };
        let name: String = name.chars().take(10).collect();
        gauge_row(
            buf,
            foot.x,
            foot.y + i as u16,
            foot.width,
            &format!("{name:<10}"),
            p,
            &th.cpu_gradient(),
            th,
            format!("{} free", fmt::bytes((m.total - m.used) as f64)),
        );
    }
}

fn temp_color_for(th: &Theme, name: &str) -> Color {
    match name {
        "CPU" => th.peach,
        "GPU" => th.green,
        "SSD" => th.sapphire,
        "Board" => th.mauve,
        "Battery" => th.yellow,
        "WiFi" => th.teal,
        _ => th.overlay1,
    }
}

fn draw_temps(f: &mut Frame, area: Rect, app: &App, snap: &Snapshot) {
    let th = app.theme();
    let hottest = snap.temps.iter().map(|t| t.current).fold(f64::NAN, f64::max);
    let mut block = panel(th, "Temperature", th.red);
    if hottest.is_finite() {
        block = block.title_top(
            Line::from(Span::styled(format!("{hottest:.0}°C "), Style::new().fg(th.temp_color(hottest)).bold()))
                .alignment(Alignment::Right),
        );
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if snap.temps.is_empty() {
        let msg = Line::from(Span::styled("no temperature sensors found", Style::new().fg(th.overlay0)))
            .alignment(Alignment::Center);
        f.buffer_mut().set_line(inner.x, inner.y + inner.height / 2, &msg, inner.width);
        return;
    }
    let legend_w = if inner.width >= 50 { 22 } else { 0 };
    let [chart_area, legend_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(legend_w)]).areas(inner);

    let w = app.window();
    let points = chart_area.width.saturating_sub(5) as usize * 2;
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for (_, s) in &app.hist.temps {
        if let Some(m) = s.min_in(w) {
            lo = lo.min(m);
        }
        hi = hi.max(s.max_in(w));
    }
    if lo > hi {
        (lo, hi) = (30.0, 80.0);
    }
    let lo = ((lo - 3.0) / 10.0).floor() * 10.0;
    let hi = ((hi + 3.0) / 10.0).ceil() * 10.0;
    let series: Vec<(Color, Vec<(f64, f64)>)> = app
        .hist
        .temps
        .iter()
        .map(|(name, s)| {
            let pts = s
                .resample(w, points)
                .into_iter()
                .enumerate()
                .filter(|(_, v)| v.is_finite())
                .map(|(i, v)| (i as f64, v))
                .collect();
            (temp_color_for(th, name), pts)
        })
        .collect();
    let datasets = series
        .iter()
        .map(|(c, pts)| {
            Dataset::default().marker(Marker::Braille).graph_type(GraphType::Line).style(Style::new().fg(*c)).data(pts)
        })
        .collect();
    let mid = (lo + hi) / 2.0;
    let chart = Chart::new(datasets)
        .style(Style::new().bg(th.base))
        .x_axis(Axis::default().bounds([0.0, points.max(1) as f64]))
        .y_axis(
            Axis::default()
                .bounds([lo, hi])
                .style(Style::new().fg(th.surface1))
                .labels(vec![
                    Span::styled(format!("{lo:.0}°"), Style::new().fg(th.overlay0)),
                    Span::styled(format!("{mid:.0}°"), Style::new().fg(th.overlay0)),
                    Span::styled(format!("{hi:.0}°"), Style::new().fg(th.overlay0)),
                ]),
        );
    f.render_widget(chart, chart_area);
    let buf = f.buffer_mut();
    time_axis(buf, chart_area.x + 4, chart_area.width.saturating_sub(4), area.bottom() - 1, app.window_secs(), th);

    if legend_w == 0 {
        return;
    }
    for (i, t) in snap.temps.iter().enumerate() {
        let y = legend_area.y + i as u16 * 2;
        if y + 1 >= legend_area.bottom() {
            break;
        }
        let peak = app.peak_temps.get(t.name).copied().unwrap_or(t.current);
        let line = Line::from(vec![
            Span::styled("● ", Style::new().fg(temp_color_for(th, t.name))),
            Span::styled(format!("{:<8}", t.name), Style::new().fg(th.subtext)),
            Span::styled(format!("{:>3.0}°", t.current), Style::new().fg(th.temp_color(t.current)).bold()),
            Span::styled(format!(" ↑{peak:.0}°"), Style::new().fg(th.overlay0)),
        ]);
        buf.set_line(legend_area.x + 1, y, &line, legend_area.width - 1);
        gradient_bar(
            buf,
            Rect { x: legend_area.x + 3, y: y + 1, width: legend_area.width.saturating_sub(4), height: 1 },
            (t.current - 20.0) / 80.0,
            &th.heat_gradient(),
            th.surface0,
        );
    }
}

fn draw_procs(buf: &mut Buffer, area: Rect, app: &mut App) {
    let th = app.theme();
    let procs = app.visible_procs();
    let total = app.snap.as_ref().map(|s| s.procs.len()).unwrap_or(0);
    let filter_line = match app.mode {
        Mode::Filter => Line::from(vec![
            Span::styled(" / ", Style::new().fg(th.yellow).bold()),
            Span::styled(format!("{}▏", app.filter), Style::new().fg(th.text)),
        ]),
        _ if !app.filter.is_empty() => Line::from(vec![
            Span::styled(" filter: ", Style::new().fg(th.overlay0)),
            Span::styled(format!("{} ", app.filter), Style::new().fg(th.yellow)),
        ]),
        _ => Line::from(Span::styled(format!(" {total} procs "), Style::new().fg(th.overlay1))),
    };
    let block = panel(th, "Processes", th.yellow)
        .title_top(filter_line.alignment(Alignment::Center))
        .title_top(
            Line::from(vec![
                Span::styled("sort ", Style::new().fg(th.overlay0)),
                Span::styled(app.sort.label(), Style::new().fg(th.peach).bold()),
                Span::styled(if app.sort_desc { " ▼ " } else { " ▲ " }, Style::new().fg(th.peach)),
            ])
            .alignment(Alignment::Right),
        );
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height < 2 {
        return;
    }

    let visible = inner.height as usize - 1;
    let selected = app.selected.min(procs.len().saturating_sub(1));
    let mut scroll = app.scroll;
    if selected < scroll {
        scroll = selected;
    } else if selected >= scroll + visible {
        scroll = selected + 1 - visible;
    }

    let wide = inner.width >= 70;
    let header_style = Style::new().fg(th.overlay1).add_modifier(Modifier::BOLD);
    let mut header = vec![Span::styled(format!(" {:>7} ", "PID"), header_style)];
    if wide {
        header.push(Span::styled(format!("{:<10} ", "USER"), header_style));
    }
    header.push(Span::styled(format!("{:<12}", "CPU%"), header_style));
    header.push(Span::styled(format!("{:>9} ", "MEM"), header_style));
    if wide {
        header.push(Span::styled(format!("{:>8} ", "TIME"), header_style));
    }
    header.push(Span::styled("NAME", header_style));
    buf.set_line(inner.x, inner.y, &Line::from(header), inner.width);

    let mem_total = app.snap.as_ref().map(|s| s.mem_total).unwrap_or(1).max(1);
    let cores = app.info.cores.max(1) as f64;
    for (row, p) in procs.iter().skip(scroll).take(visible).enumerate() {
        let y = inner.y + 1 + row as u16;
        let is_sel = scroll + row == selected;
        let base = if is_sel { Style::new().bg(th.surface0) } else { Style::new() };
        if is_sel {
            buf.set_style(Rect { x: inner.x, y, width: inner.width, height: 1 }, base);
        }
        // Mini bar: per-process CPU relative to one core, capped at 5 pips.
        let pips = ((p.cpu / 100.0) * 5.0).ceil().clamp(0.0, 5.0) as usize;
        let cpu_color = th.load_color((p.cpu / cores * 4.0).min(100.0).max(p.cpu.min(100.0)));
        let mem_p = p.mem as f64 / mem_total as f64 * 100.0;
        let name_color = if is_sel { th.text } else if p.cpu > 50.0 { th.peach } else { th.subtext };
        let mut spans = vec![
            Span::styled(if is_sel { "▌" } else { " " }, base.fg(th.mauve)),
            Span::styled(format!("{:>7} ", p.pid), base.fg(th.overlay1)),
        ];
        if wide {
            let user: String = p.user.chars().take(10).collect();
            spans.push(Span::styled(format!("{user:<10} "), base.fg(th.lavender)));
        }
        spans.push(Span::styled("■".repeat(pips), base.fg(cpu_color)));
        spans.push(Span::styled("■".repeat(5 - pips), base.fg(th.surface1)));
        spans.push(Span::styled(format!("{:>6.1} ", p.cpu), base.fg(cpu_color)));
        spans.push(Span::styled(
            format!("{:>9} ", fmt::bytes(p.mem as f64)),
            base.fg(gradient(&th.mem_gradient(), (mem_p / 10.0).min(1.0))),
        ));
        if wide {
            spans.push(Span::styled(format!("{:>8} ", fmt::cpu_time(p.run_time)), base.fg(th.overlay0)));
        }
        let mut name_style = base.fg(name_color);
        if is_sel {
            name_style = name_style.add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(p.name.clone(), name_style));
        if p.status != "Runnable" && p.status != "Run" && p.status != "Sleeping" && p.status != "Sleep" && wide {
            spans.push(Span::styled(format!(" ({})", p.status.to_lowercase()), base.fg(th.surface2)));
        }
        buf.set_line(inner.x, y, &Line::from(spans), inner.width);
    }

    // Scrollbar thumb on the right border.
    if procs.len() > visible && visible > 0 {
        let track = inner.height - 1;
        let thumb = ((visible as f64 / procs.len() as f64) * track as f64).ceil().max(1.0) as u16;
        let pos = ((scroll as f64 / (procs.len() - visible) as f64) * (track - thumb) as f64).round() as u16;
        for i in 0..thumb {
            buf.set_string(area.right() - 1, inner.y + 1 + pos + i, "┃", Style::new().fg(th.overlay0));
        }
    }

    app.selected = selected;
    app.scroll = scroll;
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_help(f: &mut Frame, area: Rect, th: &Theme) {
    let rows: &[(&str, &str)] = &[
        ("↑ ↓ / j k", "select process"),
        ("PgUp PgDn g G", "jump in process list"),
        ("← → / [ ]", "change history window"),
        ("s", "cycle sort column"),
        ("r", "reverse sort order"),
        ("/", "filter processes (Esc clears)"),
        ("x / K / Del", "send SIGTERM to selected"),
        ("t", "cycle color theme"),
        ("b", "toggle starfield background"),
        ("p / space", "pause sampling"),
        ("q / Esc", "quit"),
    ];
    let popup = centered(area, 52, rows.len() as u16 + 4);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.mauve))
        .title_top(Line::from(Span::styled(" Keys ", Style::new().fg(th.mauve).bold())))
        .style(Style::new().bg(th.mantle));
    let lines: Vec<Line> = std::iter::once(Line::raw(""))
        .chain(rows.iter().map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("  {k:>15}  "), Style::new().fg(th.peach).bold()),
                Span::styled(*v, Style::new().fg(th.text)),
            ])
        }))
        .collect();
    f.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_confirm(f: &mut Frame, area: Rect, th: &Theme, pid: u32, name: &str) {
    let popup = centered(area, 46, 6);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.red))
        .title_top(Line::from(Span::styled(" Kill process? ", Style::new().fg(th.red).bold())))
        .style(Style::new().bg(th.mantle));
    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled(name.to_string(), Style::new().fg(th.text).bold()),
            Span::styled(format!("  (pid {pid})"), Style::new().fg(th.overlay1)),
        ])
        .alignment(Alignment::Center),
        Line::from(vec![
            Span::styled("[y]", Style::new().fg(th.green).bold()),
            Span::styled(" SIGTERM    ", Style::new().fg(th.subtext)),
            Span::styled("[any]", Style::new().fg(th.overlay1).bold()),
            Span::styled(" cancel", Style::new().fg(th.subtext)),
        ])
        .alignment(Alignment::Center),
    ];
    f.render_widget(Paragraph::new(lines).block(block), popup);
}
