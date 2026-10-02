mod app;
mod collector;
mod fans;
mod fmt;
mod history;
mod snapshot;
mod stars;
mod theme;
mod ui;
mod widgets;

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};

use app::App;

const USAGE: &str = "supertop — a pretty terminal system monitor

usage: supertop [--interval <ms>] [--fps <n>] [--theme <n>] [--no-stars]

  --interval <ms>   sampling interval in milliseconds (default 1000)
  --fps <n>         animation frame rate (default 30)
  --theme <n>       start with theme index 0-3 (mocha, tokyo night, nord, dracula)
  --window <w>      initial history window: 1m, 5m, 15m, 30m or 1h (default 5m)
  --no-stars        start with the starfield background off";

struct Args {
    interval: Duration,
    fps: u32,
    theme: usize,
    stars: bool,
    window: usize,
    dump_html: Option<(String, u16, u16, u64)>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args { interval: Duration::from_millis(1000), fps: 30, theme: 0, stars: true, window: 1, dump_html: None };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut num = |name: &str| -> Result<u64> {
            it.next().and_then(|v| v.parse().ok()).ok_or_else(|| anyhow::anyhow!("{name} needs a number\n\n{USAGE}"))
        };
        match a.as_str() {
            "--interval" | "-i" => args.interval = Duration::from_millis(num("--interval")?.clamp(100, 10_000)),
            "--fps" => args.fps = num("--fps")?.clamp(1, 120) as u32,
            "--theme" => args.theme = num("--theme")? as usize % theme::THEMES.len(),
            "--no-stars" => args.stars = false,
            "--window" | "-w" => {
                let w = it.next().unwrap_or_default();
                args.window = app::WINDOWS
                    .iter()
                    .position(|(_, label)| *label == w)
                    .ok_or_else(|| anyhow::anyhow!("--window must be one of 1m, 5m, 15m, 30m, 1h"))?;
            }
            "--dump-html" => {
                let path = it.next().ok_or_else(|| anyhow::anyhow!("--dump-html needs a path"))?;
                let size = it.next().unwrap_or_else(|| "200x50".into());
                let (w, h) = size.split_once('x').ok_or_else(|| anyhow::anyhow!("size must look like 200x50"))?;
                let secs = it.next().and_then(|s| s.parse().ok()).unwrap_or(6);
                args.dump_html = Some((path, w.parse()?, h.parse()?, secs));
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("supertop {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}\n\n{USAGE}"),
        }
    }
    Ok(args)
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let mut app = App::new(collector::static_info(), args.interval.as_millis() as u64);
    app.theme_idx = args.theme;
    app.stars = args.stars;
    app.window_idx = args.window;
    if let Some((path, w, h, secs)) = &args.dump_html {
        return snapshot::dump(&mut app, args.interval, path, *w, *h, *secs);
    }
    let snapshots = collector::spawn(args.interval);

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &snapshots, args.fps);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    snapshots: &std::sync::mpsc::Receiver<collector::Snapshot>,
    fps: u32,
) -> Result<()> {
    let frame = Duration::from_secs_f64(1.0 / fps as f64);
    let mut last = Instant::now();
    let mut fps_window = (Instant::now(), 0u32);

    while !app.quit {
        while let Ok(s) = snapshots.try_recv() {
            app.ingest(s);
        }
        let now = Instant::now();
        app.animate(now.duration_since(last).as_secs_f64());
        last = now;

        terminal.draw(|f| ui::draw(f, app))?;

        fps_window.1 += 1;
        let elapsed = fps_window.0.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            app.fps = fps_window.1 as f64 / elapsed;
            fps_window = (Instant::now(), 0);
        }

        let deadline = now + frame;
        while let Some(timeout) = deadline.checked_duration_since(Instant::now()) {
            if !event::poll(timeout)? {
                break;
            }
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key);
                }
            }
        }
    }
    Ok(())
}
