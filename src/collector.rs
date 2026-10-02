//! Background sampling thread. Everything slow (process scans, SMC reads)
//! happens here so the render loop can animate smoothly.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{Components, Disks, Networks, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};

use crate::fans::{Fan, FanReader};

#[derive(Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub user: String,
    pub cpu: f64,
    pub mem: u64,
    pub run_time: u64,
    pub status: String,
}

#[derive(Clone)]
pub struct Mount {
    pub name: String,
    pub mount: String,
    pub used: u64,
    pub total: u64,
}

#[derive(Clone)]
pub struct TempGroup {
    pub name: &'static str,
    pub current: f64,
}

#[derive(Clone)]
pub struct Snapshot {
    pub cpu_total: f64,
    pub cpu_cores: Vec<f64>,
    pub cpu_freq_mhz: u64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    pub net_rx_total: u64,
    pub net_tx_total: u64,
    pub net_iface: String,
    pub disk_read_bps: f64,
    pub disk_write_bps: f64,
    pub mounts: Vec<Mount>,
    pub temps: Vec<TempGroup>,
    pub fans: Vec<Fan>,
    pub procs: Vec<ProcInfo>,
    pub load: [f64; 3],
    pub uptime: u64,
}

pub struct StaticInfo {
    pub host: String,
    pub os: String,
    pub cpu_brand: String,
    pub cores: usize,
    pub physical_cores: Option<usize>,
}

pub fn static_info() -> StaticInfo {
    let mut sys = System::new();
    sys.refresh_cpu_all();
    StaticInfo {
        host: System::host_name().unwrap_or_else(|| "localhost".into()),
        os: System::long_os_version().unwrap_or_else(|| System::name().unwrap_or_default()),
        cpu_brand: sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
        cores: sys.cpus().len(),
        physical_cores: System::physical_core_count(),
    }
}

pub fn spawn(interval: Duration) -> Receiver<Snapshot> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("collector".into())
        .spawn(move || run(tx, interval))
        .expect("spawn collector thread");
    rx
}

fn skip_iface(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "lo", "utun", "awdl", "llw", "bridge", "gif", "stf", "anpi", "ap", "docker", "veth", "br-", "virbr", "tun",
        "tap", "wg",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Buckets raw sensor labels from macOS (IOHID) and Linux (hwmon) into a few
/// human-friendly groups.
fn temp_group(label: &str) -> Option<&'static str> {
    let l = label.to_lowercase();
    if l.contains("tcal") {
        return None;
    }
    if l.contains("gpu") || l.contains("amdgpu") || l.contains("nouveau") || l.contains("edge") || l.contains("junction")
    {
        Some("GPU")
    } else if l.contains("nand") || l.contains("nvme") || l.contains("ssd") || l.contains("composite") || l.contains("drive")
    {
        Some("SSD")
    } else if l.contains("battery") || l.contains("gas gauge") || l.contains("bat") {
        Some("Battery")
    } else if l.contains("tdie")
        || l.contains("cpu")
        || l.contains("core")
        || l.contains("package")
        || l.contains("tctl")
        || l.contains("tccd")
        || l.contains("k10temp")
        || l.contains("soc")
    {
        Some("CPU")
    } else if l.contains("wifi") || l.contains("iwlwifi") || l.contains("wlan") {
        Some("WiFi")
    } else if l.contains("tdev") || l.contains("pch") || l.contains("acpi") {
        Some("Board")
    } else {
        Some("Other")
    }
}

const TEMP_ORDER: &[&str] = &["CPU", "GPU", "SSD", "Board", "Battery", "WiFi", "Other"];

fn run(tx: Sender<Snapshot>, interval: Duration) {
    let mut sys = System::new();
    let mut networks = Networks::new_with_refreshed_list();
    let mut disks = Disks::new_with_refreshed_list();
    let mut components = Components::new_with_refreshed_list();
    let users = Users::new_with_refreshed_list();
    let mut fans = FanReader::new();

    let proc_kind = ProcessRefreshKind::nothing().with_cpu().with_memory().with_user(UpdateKind::OnlyIfNotSet);
    sys.refresh_cpu_all();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, proc_kind);

    let mut last = Instant::now();
    let mut tick: u64 = 0;
    loop {
        thread::sleep(interval.saturating_sub(last.elapsed()));
        let dt = last.elapsed().as_secs_f64().max(0.001);
        last = Instant::now();

        sys.refresh_cpu_all();
        sys.refresh_memory();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, proc_kind);
        networks.refresh(true);
        // Re-scan the disk list occasionally to pick up new mounts; just refresh
        // counters the rest of the time.
        disks.refresh(tick.is_multiple_of(30));
        components.refresh(false);
        tick += 1;

        let cpu_cores: Vec<f64> = sys.cpus().iter().map(|c| c.cpu_usage() as f64).collect();
        let cpu_freq_mhz = sys.cpus().iter().map(|c| c.frequency()).max().unwrap_or(0);

        let (mut rx_b, mut tx_b, mut rx_t, mut tx_t) = (0u64, 0u64, 0u64, 0u64);
        let mut busiest = (0u64, String::new());
        for (name, data) in networks.list() {
            if skip_iface(name) {
                continue;
            }
            rx_b += data.received();
            tx_b += data.transmitted();
            rx_t += data.total_received();
            tx_t += data.total_transmitted();
            let total = data.total_received() + data.total_transmitted();
            if total > busiest.0 {
                busiest = (total, name.clone());
            }
        }

        // APFS volumes in one container report identical device counters; count
        // each physical device once.
        let mut seen = HashSet::new();
        let (mut rd, mut wr) = (0u64, 0u64);
        let mut mounts = Vec::new();
        let mut seen_space = HashSet::new();
        for d in disks.list() {
            let u = d.usage();
            let key = (d.name().to_os_string(), d.total_space(), u.total_read_bytes, u.total_written_bytes);
            if seen.insert(key) {
                rd += u.read_bytes;
                wr += u.written_bytes;
            }
            let mount = d.mount_point().to_string_lossy().to_string();
            if mount.starts_with("/System/Volumes/")
                || mount.starts_with("/private/var")
                || d.is_read_only()
                || d.total_space() == 0
            {
                continue;
            }
            if seen_space.insert((d.total_space(), d.available_space())) {
                mounts.push(Mount {
                    name: d.name().to_string_lossy().to_string(),
                    mount,
                    used: d.total_space().saturating_sub(d.available_space()),
                    total: d.total_space(),
                });
            }
        }

        let mut groups: Vec<(&'static str, f64)> = Vec::new();
        for c in components.list() {
            let Some(t) = c.temperature().map(|t| t as f64) else { continue };
            if !(-20.0..=150.0).contains(&t) || t == 0.0 {
                continue;
            }
            let Some(g) = temp_group(c.label()) else { continue };
            // The hottest sensor in a group is the one worth watching.
            match groups.iter_mut().find(|(n, _)| *n == g) {
                Some(e) => e.1 = e.1.max(t),
                None => groups.push((g, t)),
            }
        }
        groups.sort_by_key(|(n, _)| TEMP_ORDER.iter().position(|o| o == n));
        let temps = groups.into_iter().map(|(name, current)| TempGroup { name, current }).collect();

        let procs = sys
            .processes()
            .values()
            .filter(|p| p.thread_kind().is_none())
            .map(|p| ProcInfo {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().to_string(),
                user: p
                    .user_id()
                    .and_then(|u| users.get_user_by_id(u))
                    .map(|u| u.name().to_string())
                    .unwrap_or_else(|| "-".into()),
                cpu: p.cpu_usage() as f64,
                mem: p.memory(),
                run_time: p.run_time(),
                status: p.status().to_string(),
            })
            .collect();

        let load = System::load_average();
        let snap = Snapshot {
            cpu_total: sys.global_cpu_usage() as f64,
            cpu_cores,
            cpu_freq_mhz,
            mem_used: sys.used_memory(),
            mem_total: sys.total_memory(),
            swap_used: sys.used_swap(),
            swap_total: sys.total_swap(),
            net_rx_bps: rx_b as f64 / dt,
            net_tx_bps: tx_b as f64 / dt,
            net_rx_total: rx_t,
            net_tx_total: tx_t,
            net_iface: busiest.1,
            disk_read_bps: rd as f64 / dt,
            disk_write_bps: wr as f64 / dt,
            mounts,
            temps,
            fans: fans.read(),
            procs,
            load: [load.one, load.five, load.fifteen],
            uptime: System::uptime(),
        };
        if tx.send(snap).is_err() {
            return;
        }
    }
}
