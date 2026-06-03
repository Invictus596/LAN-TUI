# LAN-TUI

A cinematic terminal dashboard that visualizes the Local Area Network (LAN) as an interactive node graph.

Built with **Rust** and **ratatui**.

## Status

**Step 1 — Boilerplate complete.** State machine with 5 screens (Splash, AnimatingIntro, Graph, AnimatingZoom, Detail), mock nodes, and keyboard-driven transitions.

### Controls

| Key      | Action     |
| -------- | ---------- |
| `Enter`  | Next state |
| `q` / `Esc` | Quit    |

## Tech Stack

- **Language:** Rust
- **TUI:** ratatui + crossterm
- **Error handling:** color-eyre

## Roadmap

| # | Step | Status |
|---|------|--------|
| 1 | Boilerplate & state machine | ✅ Complete |
| 2 | Network scanning engine | ⏳ |
| 3 | 3D projection engine | ⏳ |
| 4 | Node graph rendering | ⏳ |
| 5 | Zoom & detail animations | ⏳ |

## Run

```bash
cargo run
```
