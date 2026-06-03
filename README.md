# LAN-TUI

```
/$$        /$$$$$$  /$$   /$$      /$$$$$$$$ /$$   /$$ /$$$$$$
| $$       /$$__  $$| $$$ | $$     |__  $$__/| $$  | $$|_  $$_/
| $$      | $$  \ $$| $$$$| $$        | $$   | $$  | $$  | $$  
| $$      | $$$$$$$$| $$ $$ $$ /$$$$$$| $$   | $$  | $$  | $$  
| $$      | $$__  $$| $$  $$$$|______/| $$   | $$  | $$  | $$  
| $$      | $$  | $$| $$\  $$$        | $$   | $$  | $$  | $$  
| $$$$$$$$| $$  | $$| $$ \  $$        | $$   |  $$$$$$/ /$$$$$$
|________/|__/  |__/|__/  \__/        |__/    \______/ |______/
```

A cinematic terminal dashboard that visualizes the Local Area Network (LAN) as an interactive node graph.

Built with **Rust** and **ratatui**.

## Status

**Step 2 — Background discovery engine complete.** MPSC channel connects a background scanner thread to the UI render loop. Latency values update every 2-3 seconds and display on the Graph and Detail screens with color coding.

### Controls

| Key        | Action     |
| ---------- | ---------- |
| `Enter`    | Next state |
| `q` / `Esc` | Quit      |

## Architecture

```
┌─────────────────────────┐     mpsc::channel     ┌──────────────────┐
│ Background thread       │ ──────────────────►   │ UI render loop   │
│ (simulate_scan, 2-3s)   │   tx.send(nodes)      │ rx.try_recv()    │
└─────────────────────────┘                       └──────────────────┘
```

The scanner runs on a dedicated `std::thread::spawn` worker. It jitters latency values deterministically (no external crates) and pushes updates through an MPSC channel. The main loop consumes via `try_recv()` to stay non-blocking.

## Tech Stack

- **Language:** Rust
- **TUI:** ratatui + crossterm
- **Concurrency:** std::sync::mpsc
- **Error handling:** color-eyre

## Roadmap

| # | Step | Status |
|---|------|--------|
| 1 | Boilerplate & state machine | ✅ Complete |
| 2 | Background network discovery | ✅ Complete |
| 3 | 3D projection engine | ⏳ |
| 4 | Node graph rendering | ⏳ |
| 5 | Zoom & detail animations | ⏳ |

## Run

```bash
cargo run
```
