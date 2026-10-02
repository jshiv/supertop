use std::collections::HashMap;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::collector::{ProcInfo, Snapshot, StaticInfo};
use crate::history::Series;
use crate::theme::{Theme, THEMES};

pub const WINDOWS: &[(usize, &str)] = &[(60, "1m"), (300, "5m"), (900, "15m"), (1800, "30m"), (3600, "1h")];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Cpu,
    Mem,
    Pid,
    Name,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Cpu => "CPU",
            SortKey::Mem => "MEM",
            SortKey::Pid => "PID",
            SortKey::Name => "NAME",
        }
    }
    fn next(self) -> Self {
        match self {
            SortKey::Cpu => SortKey::Mem,
            SortKey::Mem => SortKey::Pid,
            SortKey::Pid => SortKey::Name,
            SortKey::Name => SortKey::Cpu,
        }
    }
}

pub enum Mode {
    Normal,
    Filter,
    ConfirmKill(u32, String),
    Help,
}

#[derive(Default)]
pub struct History {
    pub cpu: Series,
    pub cores: Vec<Series>,
    pub mem: Series,
    pub swap: Series,
    pub net_rx: Series,
    pub net_tx: Series,
    pub disk_r: Series,
    pub disk_w: Series,
    pub temps: Vec<(&'static str, Series)>,
    pub fans: Vec<Series>,
    pub samples: usize,
}

pub struct App {
    pub info: StaticInfo,
    pub snap: Option<Snapshot>,
    pub hist: History,
    pub theme_idx: usize,
    pub window_idx: usize,
    pub sort: SortKey,
    pub sort_desc: bool,
    pub selected: usize,
    pub scroll: usize,
    pub filter: String,
    pub mode: Mode,
    pub paused: bool,
    pub stars: bool,
    pub quit: bool,
    pub started: Instant,
    pub fan_angles: Vec<f64>,
    pub fan_speeds: Vec<f64>,
    pub fps: f64,
    pub status_msg: Option<(String, Instant)>,
    pub peak_temps: HashMap<&'static str, f64>,
    pub interval_ms: u64,
}

impl App {
    pub fn new(info: StaticInfo, interval_ms: u64) -> Self {
        crate::history::set_capacity((3_600_000 / interval_ms.max(1)) as usize);
        Self {
            info,
            snap: None,
            hist: History::default(),
            theme_idx: 0,
            window_idx: 1,
            sort: SortKey::Cpu,
            sort_desc: true,
            selected: 0,
            scroll: 0,
            filter: String::new(),
            mode: Mode::Normal,
            paused: false,
            stars: true,
            quit: false,
            started: Instant::now(),
            fan_angles: Vec::new(),
            fan_speeds: Vec::new(),
            fps: 0.0,
            status_msg: None,
            peak_temps: HashMap::new(),
            interval_ms,
        }
    }

    pub fn theme(&self) -> &'static Theme {
        &THEMES[self.theme_idx]
    }

    /// The selected history window in seconds.
    pub fn window_secs(&self) -> usize {
        WINDOWS[self.window_idx].0
    }

    /// The selected history window in samples.
    pub fn window(&self) -> usize {
        (self.window_secs() as u64 * 1000 / self.interval_ms.max(1)).max(2) as usize
    }

    pub fn window_label(&self) -> &'static str {
        WINDOWS[self.window_idx].1
    }

    pub fn ingest(&mut self, s: Snapshot) {
        if self.paused {
            return;
        }
        let h = &mut self.hist;
        h.samples += 1;
        let n = h.samples;
        h.cpu.push(s.cpu_total);
        if h.cores.len() < s.cpu_cores.len() {
            h.cores.resize_with(s.cpu_cores.len(), Series::default);
        }
        for (series, v) in h.cores.iter_mut().zip(&s.cpu_cores) {
            series.push_aligned(*v, n);
        }
        h.mem.push(pct(s.mem_used, s.mem_total));
        h.swap.push(pct(s.swap_used, s.swap_total));
        h.net_rx.push(s.net_rx_bps);
        h.net_tx.push(s.net_tx_bps);
        h.disk_r.push(s.disk_read_bps);
        h.disk_w.push(s.disk_write_bps);
        for t in &s.temps {
            let idx = match h.temps.iter().position(|(name, _)| *name == t.name) {
                Some(i) => i,
                None => {
                    h.temps.push((t.name, Series::default()));
                    h.temps.len() - 1
                }
            };
            h.temps[idx].1.push_aligned(t.current, n);
            let peak = self.peak_temps.entry(t.name).or_insert(t.current);
            *peak = peak.max(t.current);
        }
        if h.fans.len() < s.fans.len() {
            h.fans.resize_with(s.fans.len(), Series::default);
        }
        for (series, f) in h.fans.iter_mut().zip(&s.fans) {
            series.push_aligned(f.rpm, n);
        }
        if self.fan_angles.len() != s.fans.len() {
            self.fan_angles = (0..s.fans.len()).map(|i| i as f64 * 0.7).collect();
            self.fan_speeds = vec![0.0; s.fans.len()];
        }
        self.snap = Some(s);
    }

    /// Advances fan rotation. Real fans spin at 20-100 rev/s, far beyond what a
    /// terminal can show without strobing, so we map min..max RPM onto a calm
    /// 0.35..3.5 rev/s and ease towards the target so speed changes feel physical.
    pub fn animate(&mut self, dt: f64) {
        let Some(s) = &self.snap else { return };
        for (i, f) in s.fans.iter().enumerate() {
            let target = if f.rpm < 1.0 {
                0.0
            } else {
                (0.35 + f.ratio().powf(0.8) * 3.15) * std::f64::consts::TAU
            };
            let speed = &mut self.fan_speeds[i];
            *speed += (target - *speed) * (1.0 - (-dt * 1.5).exp());
            self.fan_angles[i] = (self.fan_angles[i] + *speed * dt) % std::f64::consts::TAU;
        }
    }

    /// Processes after filtering and sorting, as displayed.
    pub fn visible_procs(&self) -> Vec<&ProcInfo> {
        let Some(s) = &self.snap else { return Vec::new() };
        let needle = self.filter.to_lowercase();
        let mut v: Vec<&ProcInfo> = s
            .procs
            .iter()
            .filter(|p| needle.is_empty() || p.name.to_lowercase().contains(&needle) || p.pid.to_string() == needle)
            .collect();
        match self.sort {
            SortKey::Cpu => v.sort_by(|a, b| a.cpu.total_cmp(&b.cpu).then(a.mem.cmp(&b.mem))),
            SortKey::Mem => v.sort_by_key(|p| p.mem),
            SortKey::Pid => v.sort_by_key(|p| p.pid),
            SortKey::Name => v.sort_by_key(|p| p.name.to_lowercase()),
        }
        if self.sort_desc {
            v.reverse();
        }
        v
    }

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.status_msg = Some((msg.into(), Instant::now()));
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match &self.mode {
            Mode::Help => self.mode = Mode::Normal,
            Mode::ConfirmKill(pid, name) => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter) {
                    let (pid, name) = (*pid, name.clone());
                    let msg = match kill(pid) {
                        Ok(()) => format!("Sent SIGTERM to {name} ({pid})"),
                        Err(e) => format!("Could not signal {name} ({pid}): {e}"),
                    };
                    self.flash(msg);
                }
                self.mode = Mode::Normal;
            }
            Mode::Filter => match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.mode = Mode::Normal;
                }
                KeyCode::Enter => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.selected = 0;
                }
                _ => {}
            },
            Mode::Normal => self.on_normal_key(key),
        }
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        let len = self.visible_procs().len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('?') | KeyCode::Char('h') => self.mode = Mode::Help,
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(len.saturating_sub(1)),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::PageDown => self.selected = (self.selected + 10).min(len.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.selected = len.saturating_sub(1),
            KeyCode::Left | KeyCode::Char('[') => self.window_idx = self.window_idx.saturating_sub(1),
            KeyCode::Right | KeyCode::Char(']') => self.window_idx = (self.window_idx + 1).min(WINDOWS.len() - 1),
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.sort_desc = !matches!(self.sort, SortKey::Pid | SortKey::Name);
            }
            KeyCode::Char('r') => self.sort_desc = !self.sort_desc,
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char('t') => {
                self.theme_idx = (self.theme_idx + 1) % THEMES.len();
                self.flash(format!("Theme: {}", self.theme().name));
            }
            KeyCode::Char('b') => self.stars = !self.stars,
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                self.paused = !self.paused;
                self.flash(if self.paused { "Paused" } else { "Resumed" });
            }
            KeyCode::Char('x') | KeyCode::Char('K') | KeyCode::Delete => {
                if let Some(p) = self.visible_procs().get(self.selected) {
                    self.mode = Mode::ConfirmKill(p.pid, p.name.clone());
                }
            }
            _ => {}
        }
    }
}

pub fn pct(used: u64, total: u64) -> f64 {
    if total == 0 { 0.0 } else { used as f64 / total as f64 * 100.0 }
}

#[cfg(unix)]
fn kill(pid: u32) -> std::io::Result<()> {
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

#[cfg(not(unix))]
fn kill(_pid: u32) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "not supported on this platform"))
}
