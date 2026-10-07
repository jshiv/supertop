# supertop

A good-looking terminal system monitor in Rust, in the spirit of `htop`, with
history charts for CPU, memory, network, disk and temperature, plus your
machine's fans drawn as spinning blue case fans driven by their real RPM.

Visual style borrows from [superseedr](https://github.com/Jagalite/superseedr):
Catppuccin colors, braille charts, a twinkling starfield and a keybind footer.

## Install

Prebuilt binaries for macOS and Linux (x86_64 and arm64):

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jshiv/supertop/releases/latest/download/supertop-installer.sh | sh
```

Or download an archive from the [releases page](https://github.com/jshiv/supertop/releases).

## Run from source

```sh
cargo run --release
# or install it
cargo install --path . && supertop
```

## Releasing

Releases are built by [cargo-dist](https://opensource.axo.dev/cargo-dist/). Bump
`version` in `Cargo.toml`, then push a matching tag:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

The `Release` workflow builds every target and publishes a GitHub Release with
the archives and the installer script.

## Options

```
--interval <ms>   sampling interval in milliseconds (default 1000)
--fps <n>         animation frame rate (default 30)
--theme <n>       0 Catppuccin Mocha, 1 Tokyo Night, 2 Nord, 3 Dracula
--window <w>      initial history window: 1m, 5m, 15m, 30m, 1h (default 5m)
--no-stars        start with the starfield off
```

## Panels

| Panel | What it shows |
|---|---|
| **CPU** | Total usage over the selected window as a braille area chart, shaded green to red by height |
| **Cores** | One sparkline and current % per logical core |
| **Fans** | One animated fan per physical fan. Spin speed follows real RPM (mapped into a range a terminal can show without strobing), with motion blur and a brighter glow as it speeds up |
| **Memory** | RAM % history, plus RAM and swap gauges |
| **Network** | Download (up) and upload (down) mirrored around a center line, each autoscaled |
| **Disk** | Read/write throughput mirrored the same way, plus free space per volume |
| **Temperature** | Sensors grouped into CPU / GPU / SSD / Battery / …, showing the hottest sensor in each group, with session peaks |
| **Processes** | Sortable, filterable process list; send SIGTERM from the UI |

## Keys

| Key | Action |
|---|---|
| `↑ ↓` / `j k`, `PgUp PgDn`, `g G` | select process |
| `← →` / `[ ]` | change history window (1m … 1h) |
| `s` / `r` | cycle sort column / reverse |
| `/` | filter processes by name or PID (`Esc` clears) |
| `x` / `K` / `Del` | SIGTERM the selected process (asks first) |
| `t` | cycle theme |
| `b` | toggle starfield |
| `p` / `space` | pause sampling |
| `?` | help |
| `q` / `Esc` | quit |

## Platform notes

- **Fans:** on macOS they're read directly from the AppleSMC (`F{n}Ac`, `F{n}Mn`, `F{n}Mx`
  keys), with no root needed. On Linux they come from `/sys/class/hwmon/*/fan*_input`. Apple
  Silicon fans can stop completely at idle, and those show as `idle`. Fanless machines
  show a still fan and "passive cooling".
- **Disks:** APFS volumes that share one physical device are only counted once in I/O.
  Read-only images (mounted DMGs) are left out of the free-space list.
- **Network:** loopback, VPN tunnels (`utun`, `tun`, `wg`), and bridge/virtual interfaces
  are skipped so traffic isn't counted twice.

## Previews without a terminal

`supertop --dump-html out.html 200x52 [seconds]` samples for a few seconds and writes
one rendered frame as colored HTML, which is handy for screenshots and for checking layouts.

## Layout of the code

```
src/
  main.rs       args, terminal setup, 30 fps render loop
  collector.rs  background sampling thread (sysinfo + fans) → Snapshot
  fans.rs       macOS SMC / Linux hwmon fan readers
  app.rs        state, history ingestion, fan physics, key handling
  history.rs    1-hour ring buffers with peak-preserving resampling
  ui.rs         layout and every panel
  widgets.rs    braille area graph, gradient bars, sparklines, FanArt
  stars.rs      twinkling starfield and the occasional shooting star
  theme.rs      palettes and gradient helpers
  snapshot.rs   --dump-html offscreen renderer
```
