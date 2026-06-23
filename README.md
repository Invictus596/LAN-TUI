# LAN-TUI

A cinematic terminal dashboard that visualises the Local Area Network as an interactive 3D-projected node graph.

![Screenshot](screenshot.png)

## Overview

LAN-TUI renders a live LAN topology as a 3D-projected node graph with real-time latency monitoring, smooth zoom transitions, and force-directed layout. A background scanner thread pushes network updates through an MPSC channel while the UI loop renders at 60 fps using a non-blocking event poll.

## Features

- **Network scanning** — parses `/proc/net/arp` for live devices and TCP-pings each host for latency
- **Subnet sweep** — auto-discovers hosts across the full /24 subnet on every scan cycle
- **Periodic refresh** — background scanner re-checks every 5 seconds
- **Force-directed layout** — nodes are positioned organically via a physics simulation (repulsion + attraction)
- **Live traffic monitor** — reads `/sys/class/net/<iface>/statistics` for real-time RX/TX throughput
- **Node search** — press `/` to filter nodes by name or IP in the detail panel
- **Interactive selection** — `←`/`→` keys, mouse click, or search to select nodes
- **Mouse support** — click-to-select, scroll-to-zoom
- **Latency history** — per-node sparkline showing the last 20 samples
- **Config file** — define node IPs, kinds, and positions in `lan-tui.toml`
- **Help overlay** — press `?` or `F1` for a full keybind reference

## Controls

| Key               | Action                       |
| ----------------- | ---------------------------- |
| `Enter`           | Next state / confirm search  |
| `←` / `→`         | Select previous / next node  |
| `/`               | Search/filter nodes          |
| `r`               | Trigger re-scan              |
| `?` / `F1`        | Toggle help overlay          |
| `q` / `Esc`       | Quit / cancel search / close |

| Mouse              | Action              |
| ------------------ | ------------------- |
| Left click         | Select node         |
| Scroll up / down   | Zoom in / out       |

## Architecture

```
┌──────────────────────────┐     mpsc::channel     ┌──────────────────┐
│ Background thread        │ ──────────────────►   │ UI render loop   │
│ (ARP + subnet sweep +    │   tx.send(nodes)      │ rx.try_recv()    │
│  TCP ping, every 5s)     │                       │ ~60fps event.poll│
└──────────────────────────┘                       └──────────────────┘
```

- **Scanner** runs on `std::thread::spawn`, reads ARP entries, sweeps the /24 subnet, and TCP-pings all discovered hosts.
- **Render loop** polls input at ~60 fps, consumes scanner updates via `try_recv()`, drives animations, and runs force-directed layout iterations each frame.
- **Projection** maps 3D world-space `(x, y, z)` onto a 2D terminal canvas with configurable camera scale and look-at target.
- **Force layout** applies Coulomb repulsion between all nodes and Hooke attraction along edges, clamping step sizes for stability.

## Tech Stack

- **Language:** Rust
- **TUI:** ratatui + crossterm
- **Concurrency:** std::sync::mpsc
- **Error handling:** color-eyre

## State Machine

```
Splash → AnimatingIntro → Graph → AnimatingZoom → Detail → Splash
```

## Run

```bash
cargo run
```

Configure custom nodes in `lan-tui.toml` (optional — falls back to mock data).
