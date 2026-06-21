use std::collections::HashMap;
use std::f64::consts::PI;
use std::fs;
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEventKind,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Context, Line as CanvasLine};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;
use serde::Deserialize;

const ZOOM_DURATION: f64 = 60.0;
const INTRO_DURATION: f64 = 90.0;
const MAX_HISTORY: usize = 20;

#[derive(Clone, Debug)]
struct Node {
    name: String,
    ip: String,
    kind: String,
    latency_ms: f64,
    x: f64,
    y: f64,
    z: f64,
    history: Vec<f64>,
}

impl Node {
    fn from_config(name: &str, ip: &str, kind: &str, x: f64, y: f64, z: f64) -> Self {
        Self {
            name: name.to_string(),
            ip: ip.to_string(),
            kind: kind.to_string(),
            latency_ms: 0.0,
            x, y, z,
            history: Vec::new(),
        }
    }

    fn new_unknown(ip: &str) -> Self {
        let seed: f64 = ip.split('.').last().and_then(|o| o.parse().ok()).unwrap_or(1) as f64;
        let angle = seed * 1.2;
        Self {
            name: format!("Unknown-{}", seed as u64),
            ip: ip.to_string(),
            kind: "Unknown".to_string(),
            latency_ms: 0.0,
            x: angle.cos() * 6.0,
            y: angle.sin() * 5.0,
            z: 1.0,
            history: Vec::new(),
        }
    }

    fn color(&self) -> Color {
        match self.kind.as_str() {
            "Gateway" => Color::Magenta,
            "Client" => Color::Cyan,
            "Peripheral" => Color::Yellow,
            _ => Color::White,
        }
    }

    fn push_latency(&mut self, ms: f64) {
        self.latency_ms = ms;
        self.history.push(ms);
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
    }

    fn sparkline(&self) -> String {
        if self.history.len() < 2 {
            return String::new();
        }
        let min = self.history.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = self.history.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let range = (max - min).max(0.5);
        let bars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        self.history.iter().map(|v| {
            let idx = ((v - min) / range * 7.0).round() as usize;
            bars[idx.min(7)]
        }).collect()
    }
}

#[derive(Deserialize)]
struct NodeDef {
    ip: String,
    kind: String,
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Deserialize)]
struct Config {
    nodes: HashMap<String, NodeDef>,
}

fn load_config(path: &Path) -> HashMap<String, NodeDef> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<Config>(&s).ok())
        .map(|c| c.nodes)
        .unwrap_or_default()
}

fn parse_arp_entries() -> Vec<(String, String)> {
    fs::read_to_string("/proc/net/arp")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 && parts[2] == "0x2" {
                Some((parts[0].to_string(), parts[3].to_string()))
            } else {
                None
            }
        })
        .collect()
}

fn tcp_ping(ip: &str) -> Option<f64> {
    let addr: SocketAddr = format!("{}:80", ip).parse().ok()?;
    let start = Instant::now();
    TcpStream::connect_timeout(&addr, Duration::from_millis(800)).ok()?;
    Some(start.elapsed().as_secs_f64() * 1000.0)
}

fn spawn_scanner(
    tx: mpsc::Sender<Vec<Node>>,
    config_nodes: HashMap<String, NodeDef>,
) {
    let mut known: Vec<Node> = config_nodes
        .iter()
        .map(|(name, def)| Node::from_config(name, &def.ip, &def.kind, def.x, def.y, def.z))
        .collect();

    thread::spawn(move || {
        let mut tick = 0u64;
        loop {
            thread::sleep(Duration::from_secs(2 + (tick % 2)));
            tick += 1;

            let entries = parse_arp_entries();
            let arp_ips: HashMap<&str, &str> = entries
                .iter()
                .map(|(ip, _mac)| (ip.as_str(), ip.as_str()))
                .collect();

            for node in &mut known {
                if arp_ips.contains_key(node.ip.as_str()) {
                    if let Some(ms) = tcp_ping(&node.ip) {
                        node.push_latency(ms);
                    } else {
                        node.latency_ms = 0.0;
                    }
                } else {
                    node.latency_ms = 0.0;
                }
            }

            let existing_ips: std::collections::HashSet<String> =
                known.iter().map(|n| n.ip.clone()).collect();
            let new_entries = parse_arp_entries();
            for (ip, _mac) in &new_entries {
                if !existing_ips.contains(ip) {
                    let mut n = Node::new_unknown(ip);
                    if let Some(ms) = tcp_ping(ip) {
                        n.push_latency(ms);
                    }
                    known.push(n);
                }
            }

            if tx.send(known.clone()).is_err() {
                break;
            }
        }
    });
}

struct Camera {
    target_idx: usize,
    zoom_progress: f64,
    base_scale: f64,
    zoom_scale: f64,
}

impl Camera {
    fn new() -> Self {
        Self { target_idx: 0, zoom_progress: 0.0, base_scale: 1.0, zoom_scale: 3.5 }
    }

    fn scale(&self) -> f64 {
        self.base_scale + self.zoom_progress * (self.zoom_scale - self.base_scale)
    }

    fn look_at_x(&self, nodes: &[Node]) -> f64 {
        let t = nodes.get(self.target_idx).map(|n| n.x).unwrap_or(0.0);
        self.zoom_progress * t
    }

    fn look_at_y(&self, nodes: &[Node]) -> f64 {
        let t = nodes.get(self.target_idx).map(|n| n.y).unwrap_or(0.0);
        self.zoom_progress * t
    }

    fn reset(&mut self) {
        self.zoom_progress = 0.0;
    }
}

fn project(node: &Node, look_at_x: f64, look_at_y: f64, scale: f64) -> (f64, f64) {
    ((node.x - look_at_x) * scale, (node.y - look_at_y) * scale)
}

fn edge_indices() -> Vec<(usize, usize)> {
    vec![(0, 1), (0, 2), (0, 3)]
}

fn neighbors_of(idx: usize) -> Vec<usize> {
    edge_indices()
        .into_iter()
        .filter_map(|(a, b)| {
            if a == idx { Some(b) } else if b == idx { Some(a) } else { None }
        })
        .collect()
}

struct App {
    state: StateMachine,
    nodes: Vec<Node>,
    last_update: String,
    scan_count: u64,
    camera: Camera,
    selected_idx: usize,
    pulse_phase: f64,
    intro_progress: f64,
    show_help: bool,
    content_area: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum StateMachine {
    Splash,
    AnimatingIntro,
    Graph,
    AnimatingZoom,
    Detail,
}

impl StateMachine {
    fn transition(self) -> Self {
        match self {
            Self::Splash => Self::AnimatingIntro,
            Self::AnimatingIntro => Self::Graph,
            Self::Graph => Self::AnimatingZoom,
            Self::AnimatingZoom => Self::Detail,
            Self::Detail => Self::Splash,
        }
    }

    fn label(&self) -> &str {
        match self {
            Self::Splash => "SPLASH",
            Self::AnimatingIntro => "ANIMATING INTRO",
            Self::Graph => "GRAPH",
            Self::AnimatingZoom => "ANIMATING ZOOM",
            Self::Detail => "DETAIL",
        }
    }
}

impl App {
    fn new() -> Self {
        Self {
            state: StateMachine::Splash,
            nodes: Vec::new(),
            last_update: String::from("—"),
            scan_count: 0,
            camera: Camera::new(),
            selected_idx: 0,
            pulse_phase: 0.0,
            intro_progress: 0.0,
            show_help: false,
            content_area: Rect::new(0, 0, 0, 0),
        }
    }

    fn advance(&mut self) {
        match self.state {
            StateMachine::Graph => {
                let next = (self.camera.target_idx + 1) % self.nodes.len().max(1);
                self.camera.target_idx = next;
                self.camera.zoom_progress = 0.0;
                self.state = self.state.transition();
            }
            StateMachine::AnimatingZoom => {
                self.camera.zoom_progress = 1.0;
                self.state = self.state.transition();
            }
            StateMachine::Detail => {
                self.camera.reset();
                self.state = self.state.transition();
            }
            _ => {
                self.state = self.state.transition();
            }
        }
    }

    fn update(&mut self) {
        self.pulse_phase = (self.pulse_phase + 0.06) % (PI * 2.0);
        match self.state {
            StateMachine::AnimatingZoom => {
                self.camera.zoom_progress =
                    (self.camera.zoom_progress + 1.0 / ZOOM_DURATION).min(1.0);
                if self.camera.zoom_progress >= 1.0 {
                    self.state = StateMachine::Detail;
                }
            }
            StateMachine::AnimatingIntro => {
                self.intro_progress =
                    (self.intro_progress + 1.0 / INTRO_DURATION).min(1.0);
            }
            _ => {}
        }
    }
}

fn main() -> io::Result<()> {
    color_eyre::install().ok();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let config_nodes = load_config(Path::new("lan-tui.toml"));
    let (tx, rx) = mpsc::channel();

    if config_nodes.is_empty() {
        let mock = vec![
            Node::from_config("Router", "192.168.1.1", "Gateway", 0.0, 0.0, 0.0),
            Node::from_config("Laptop", "192.168.1.42", "Client", 5.0, 3.0, 1.0),
            Node::from_config("Phone", "192.168.1.67", "Client", -4.0, 4.5, 0.5),
            Node::from_config("Printer", "192.168.1.15", "Peripheral", 3.5, -5.0, 1.2),
        ];
        let tx2 = tx.clone();
        thread::spawn(move || {
            let mut nodes = mock;
            let mut tick = 0u64;
            loop {
                thread::sleep(Duration::from_secs(2 + (tick % 2)));
                tick += 1;
                for n in &mut nodes {
                    let jitter = ((tick.wrapping_mul(7).wrapping_add(n.name.len() as u64 * 13)) % 30) as f64 * 0.3;
                    let delta = jitter - 4.5;
                    let next = (n.latency_ms + delta).clamp(0.5, 99.9);
                    let new_val = (next * 10.0).round() / 10.0;
                    n.push_latency(new_val);
                }
                if tx2.send(nodes.clone()).is_err() {
                    break;
                }
            }
        });
    } else {
        spawn_scanner(tx, config_nodes);
    }

    let mut app = App::new();
    let mut running = true;

    while running {
        if let Ok(fresh) = rx.try_recv() {
            app.nodes = fresh;
            app.scan_count += 1;
            app.last_update = format!("last scan: {}", humantime_since_epoch());
        }
        app.update();
        terminal.draw(|f| {
            let area = f.area();
            let ca = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)])
                .split(area)[1];
            app.content_area = ca;
            ui(f, &app);
        })?;
        running = handle_events(&mut app)?;
    }

    disable_raw_mode()?;
    terminal.backend_mut().execute(DisableMouseCapture)?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn humantime_since_epoch() -> String {
    let since = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since.as_secs() % 86400;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn handle_events(app: &mut App) -> io::Result<bool> {
    if event::poll(Duration::from_millis(16))? {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Enter => {
                    if app.show_help {
                        app.show_help = false;
                    } else {
                        app.advance();
                    }
                }
                KeyCode::Char('?') | KeyCode::F(1) => {
                    app.show_help = !app.show_help;
                }
                KeyCode::Char('q') | KeyCode::Esc => {
                    if app.show_help {
                        app.show_help = false;
                    } else {
                        return Ok(false);
                    }
                }
                KeyCode::Left => {
                    if !app.show_help && !app.nodes.is_empty() {
                        app.selected_idx = if app.selected_idx == 0 {
                            app.nodes.len() - 1
                        } else {
                            app.selected_idx - 1
                        };
                    }
                }
                KeyCode::Right => {
                    if !app.show_help && !app.nodes.is_empty() {
                        app.selected_idx = (app.selected_idx + 1) % app.nodes.len();
                    }
                }
                _ => {}
            },
            Event::Mouse(m) => handle_mouse(app, m),
            _ => {}
        }
    }
    Ok(true)
}

fn handle_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if app.show_help {
                return;
            }
            let col = m.column;
            let row = m.row;
            let ca = app.content_area;

            if row < ca.top() || row >= ca.bottom() || col < ca.left() || col >= ca.right() {
                return;
            }

            match app.state {
                StateMachine::Detail => {
                    let rel_row = row - ca.y;
                    let header_rows = 2u16;
                    if rel_row >= header_rows {
                        let idx = (rel_row - header_rows) as usize;
                        if idx < app.nodes.len() {
                            app.selected_idx = idx;
                        }
                    }
                }
                StateMachine::Graph => {
                    if app.nodes.is_empty() {
                        return;
                    }
                    let (ca_w, ca_h) = (ca.width.max(1), ca.height.max(1));
                    let rel_col = (col - ca.x) as f64 / ca_w as f64;
                    let rel_row = (row - ca.y) as f64 / ca_h as f64;

                    let scale = app.camera.scale();
                    let look_x = app.camera.look_at_x(&app.nodes);
                    let look_y = app.camera.look_at_y(&app.nodes);
                    let extent = 12.0 / scale;

                    let canvas_x = -extent + look_x + rel_col * 2.0 * extent;
                    let canvas_y = extent + look_y - rel_row * 2.0 * extent;

                    let mut best = 0;
                    let mut best_dist = f64::INFINITY;
                    for (i, node) in app.nodes.iter().enumerate() {
                        let dx = node.x - canvas_x;
                        let dy = node.y - canvas_y;
                        let d = dx * dx + dy * dy;
                        if d < best_dist {
                            best_dist = d;
                            best = i;
                        }
                    }
                    let threshold = (6.0 / scale).max(1.0);
                    if best_dist.sqrt() < threshold {
                        app.selected_idx = best;
                    }
                }
                _ => {}
            }
        }
        MouseEventKind::ScrollUp => {
            if app.state == StateMachine::Graph {
                app.camera.base_scale = (app.camera.base_scale + 0.3).min(5.0);
            }
        }
        MouseEventKind::ScrollDown => {
            if app.state == StateMachine::Graph {
                app.camera.base_scale = (app.camera.base_scale - 0.3).max(0.5);
            }
        }
        _ => {}
    }
}

fn ui(frame: &mut ratatui::Frame, app: &App) {
    let area = frame.area();
    render_status_bar(frame, area, app);
    render_main_content(frame, area, app);
    if app.show_help {
        render_help_overlay(frame, area);
    }
}

fn render_help_overlay(frame: &mut ratatui::Frame, area: Rect) {
    let w = area.width.min(56);
    let h = 13;
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let overlay = Rect::new(x, y, w, h);

    let lines = vec![
        Line::from(Span::styled("  HELP", Style::new().fg(Color::Cyan).bold())),
        Line::from(""),
        Line::from(vec![Span::styled("  ← / →", Style::new().fg(Color::Cyan).bold()), Span::raw("  Select node")]),
        Line::from(vec![Span::styled("  Enter", Style::new().fg(Color::Cyan).bold()), Span::raw("    Advance state / close help")]),
        Line::from(vec![Span::styled("  ?  / F1", Style::new().fg(Color::Cyan).bold()), Span::raw("  Toggle this help")]),
        Line::from(vec![Span::styled("  q  / Esc", Style::new().fg(Color::Cyan).bold()), Span::raw(" Quit / close help")]),
        Line::from(""),
        Line::from(vec![Span::styled("  States:", Style::new().fg(Color::Yellow).bold())]),
        Line::from("  Splash → AnimatingIntro → Graph → AnimatingZoom → Detail"),
        Line::from(""),
        Line::from(Span::styled("  Press Enter or Esc to close", Style::new().fg(Color::DarkGray))),
    ];

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" ⌨ ").style(Style::new().fg(Color::Cyan)))
        .style(Style::new().bg(Color::Black));
    frame.render_widget(paragraph, overlay);
}

fn render_status_bar(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let layout = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]);
    let [main_area, status_area] = layout.areas(area);

    let node_count = app.nodes.len();
    let avg: f64 = if node_count == 0 {
        0.0
    } else {
        app.nodes.iter().map(|n| n.latency_ms).sum::<f64>() / node_count as f64
    };
    let alive = app.nodes.iter().filter(|n| n.latency_ms > 0.0).count();

    let status = Line::from(vec![
        Span::styled(
            format!(" STATE: {} ", app.state.label()),
            Style::new().fg(Color::White).bg(Color::DarkGray),
        ),
        Span::raw(" — "),
        Span::styled(&app.last_update, Style::new().fg(Color::Green)),
        Span::raw("  "),
        Span::styled(format!("nodes: {}", node_count), Style::new().fg(Color::Blue)),
        Span::raw("  "),
        Span::styled(format!("alive: {}", alive), Style::new().fg(Color::Green)),
        Span::raw("  "),
        Span::styled(format!("avg: {:.1}ms", avg), Style::new().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled("←/→", Style::new().fg(Color::Cyan).bold()),
        Span::raw(" · "),
        Span::styled("Enter", Style::new().fg(Color::Cyan).bold()),
        Span::raw(" · "),
        Span::styled("?", Style::new().fg(Color::Cyan).bold()),
        Span::raw(" · "),
        Span::styled("q", Style::new().fg(Color::Cyan).bold()),
    ]);
    frame.render_widget(
        Paragraph::new(status)
            .block(Block::default().borders(Borders::TOP).style(Style::new().fg(Color::Gray))),
        status_area,
    );

    frame.render_widget(Block::default(), main_area);
}

fn render_main_content(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let layout = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]);
    let [_, content_area] = layout.areas(area);

    match app.state {
        StateMachine::Splash => render_splash(frame, content_area),
        StateMachine::AnimatingIntro => render_intro_canvas(frame, content_area, app),
        StateMachine::Graph => render_graph_canvas(frame, content_area, app),
        StateMachine::AnimatingZoom => render_zoom_canvas(frame, content_area, app),
        StateMachine::Detail => render_detail_panel(frame, content_area, app),
    }
}

fn render_splash(frame: &mut ratatui::Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("LAN-TUI", Style::new().fg(Color::Magenta).bold().underlined())),
        Line::from(""),
        Line::from("A terminal-based LAN visualizer"),
        Line::from(""),
        Line::from(Span::styled("Press Enter to begin", Style::new().fg(Color::Cyan))),
        Line::from(""),
        Line::from(vec![
            Span::styled("?", Style::new().fg(Color::DarkGray)),
            Span::raw(" for help"),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_node_dot(ctx: &mut Context<'_>, x: f64, y: f64, color: Color) {
    struct Dot(f64, f64, Color);
    impl ratatui::widgets::canvas::Shape for Dot {
        fn draw(&self, p: &mut ratatui::widgets::canvas::Painter) {
            if let Some((px, py)) = p.get_point(self.0, self.1) {
                p.paint(px, py, self.2);
            }
        }
    }
    ctx.draw(&Dot(x, y, color));
}

fn render_intro_canvas(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let t = app.intro_progress;
    let scan_y = 8.0 - t * 16.0;
    let extent = 12.0;

    let canvas = Canvas::default()
        .x_bounds([-extent, extent])
        .y_bounds([-extent, extent])
        .paint(|ctx| {
            ctx.print(0.0, 9.0, "SCANNING NETWORK");
            ctx.print(0.0, -9.0, format!("[{:.0}%]", t * 100.0));

            ctx.draw(&CanvasLine {
                x1: -extent, y1: scan_y, x2: extent, y2: scan_y, color: Color::Green,
            });

            for (i, node) in app.nodes.iter().enumerate() {
                let reveal = ((t * 120.0 - i as f64 * 20.0) / 20.0).clamp(0.0, 1.0);
                if reveal <= 0.0 { continue; }
                let alpha = if reveal < 1.0 { (reveal * 3.0).min(1.0) } else { 1.0 };
                let past_scan = scan_y <= node.y;
                if past_scan || reveal > 0.0 {
                    let c = if alpha < 0.5 { Color::DarkGray } else { node.color() };
                    draw_node_dot(ctx, node.x, node.y, c);
                    ctx.print(node.x, node.y + 0.6, node.name.clone());
                    if past_scan && alpha > 0.8 {
                        ctx.print(node.x, node.y - 0.6, format!("{:.1}ms", node.latency_ms));
                    }
                }
            }
        });

    frame.render_widget(canvas, area);

    let msg = if t >= 1.0 {
        Paragraph::new(Line::from(Span::styled(
            "Scan complete — Press Enter to continue",
            Style::new().fg(Color::Green),
        ))).alignment(Alignment::Center)
    } else {
        let bar_width: usize = 20;
        let filled = (t * bar_width as f64).round() as usize;
        let remain = bar_width.saturating_sub(filled);
        let bar = format!("[{}{}]", "■".repeat(filled), "·".repeat(remain));
        Paragraph::new(Line::from(Span::styled(bar, Style::new().fg(Color::Cyan))))
            .alignment(Alignment::Center)
    };
    frame.render_widget(msg, Rect::new(area.x, area.bottom().saturating_sub(3), area.width, 1));
}

fn draw_edges(ctx: &mut Context<'_>, nodes: &[Node], selected: usize, highlight: bool) {
    for (i, j) in &edge_indices() {
        if let (Some(a), Some(b)) = (nodes.get(*i), nodes.get(*j)) {
            let is_connected = *i == selected || *j == selected;
            let color = if highlight && is_connected { Color::Cyan } else { Color::DarkGray };
            ctx.draw(&CanvasLine { x1: a.x, y1: a.y, x2: b.x, y2: b.y, color });
            if is_connected || !highlight {
                let mx = (a.x + b.x) / 2.0;
                let my = (a.y + b.y) / 2.0;
                ctx.print(mx, my, format!("{:.1}ms", (a.latency_ms + b.latency_ms) / 2.0));
            }
        }
    }
}

fn render_graph_canvas(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let scale = app.camera.scale();
    let look_at_x = app.camera.look_at_x(&app.nodes);
    let look_at_y = app.camera.look_at_y(&app.nodes);
    let extent = 12.0 / scale;
    let sel = app.selected_idx.min(app.nodes.len().max(1) - 1);
    let pulse = app.pulse_phase.sin() * 0.3 + 0.7;

    let canvas = Canvas::default()
        .x_bounds([-extent + look_at_x, extent + look_at_x])
        .y_bounds([-extent + look_at_y, extent + look_at_y])
        .paint(|ctx| {
            draw_edges(ctx, &app.nodes, sel, true);
            for (i, node) in app.nodes.iter().enumerate() {
                let is_selected = i == sel;
                let dot_color = if is_selected {
                    Color::LightCyan
                } else if pulse > 0.85 && !node.history.is_empty() {
                    Color::LightBlue
                } else {
                    node.color()
                };
                draw_node_dot(ctx, node.x, node.y, dot_color);
                let label = if is_selected {
                    format!("▶ {} ◀", node.name)
                } else {
                    format!("◉ {}", node.name)
                };
                ctx.print(node.x, node.y - 0.6, label);
                ctx.print(node.x, node.y - 1.2, format!("{:.1}ms", node.latency_ms));
                if is_selected {
                    ctx.print(node.x, node.y + 0.6, format!("{}  {}", node.ip, node.kind));
                }
            }
        });

    frame.render_widget(canvas, area);
}

fn render_zoom_canvas(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let scale = app.camera.scale();
    let look_at_x = app.camera.look_at_x(&app.nodes);
    let look_at_y = app.camera.look_at_y(&app.nodes);
    let extent = 12.0 / scale;
    let sel = app.selected_idx.min(app.nodes.len().max(1) - 1);

    let canvas = Canvas::default()
        .x_bounds([-extent + look_at_x, extent + look_at_x])
        .y_bounds([-extent + look_at_y, extent + look_at_y])
        .paint(|ctx| {
            draw_edges(ctx, &app.nodes, sel, false);
            for node in &app.nodes {
                let dx = node.x - look_at_x;
                let dy = node.y - look_at_y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < 8.0 {
                    draw_node_dot(ctx, node.x, node.y, node.color());
                }
                ctx.print(node.x, node.y - 0.6, node.name.clone());
                ctx.print(node.x, node.y + 0.4, format!("{:.1}ms", node.latency_ms));
            }
        });

    frame.render_widget(canvas, area);
    let target = &app.nodes[app.camera.target_idx.min(app.nodes.len().max(1) - 1)];
    let overlay = Paragraph::new(Line::from(vec![
        Span::styled(format!("ZOOMING → {}", target.name), Style::new().fg(Color::Yellow).bold()),
        Span::raw("  "),
        Span::styled(format!("{:.0}%", app.camera.zoom_progress * 100.0), Style::new().fg(Color::Cyan)),
    ])).alignment(Alignment::Center);
    let top = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(overlay, top);
}

fn render_detail_panel(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let sel = app.selected_idx.min(app.nodes.len().max(1) - 1);
    let chunks = Layout::horizontal([Constraint::Length(34), Constraint::Min(10)]).split(area);
    let [left, right] = [chunks[0], chunks[1]];

    let mut list_lines = vec![
        Line::from(Span::styled("NODES", Style::new().fg(Color::Cyan).bold())),
        Line::from(Span::styled("───".repeat(17), Style::new().fg(Color::DarkGray))),
    ];
    for (i, node) in app.nodes.iter().enumerate() {
        let marker = if i == sel { "▶" } else { " " };
        let c = if i == sel { Color::LightCyan } else { Color::White };
        let lc = if node.latency_ms <= 0.0 {
            Color::DarkGray
        } else if node.latency_ms < 2.0 {
            Color::Green
        } else if node.latency_ms < 8.0 {
            Color::Yellow
        } else {
            Color::Red
        };
        let lat = if node.latency_ms <= 0.0 {
            "offline".to_string()
        } else {
            format!("{:.1}ms", node.latency_ms)
        };
        list_lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(Color::Yellow).bold()),
            Span::raw(" "),
            Span::styled(&node.name, Style::new().fg(c).bold()),
            Span::raw("  "),
            Span::styled(lat, Style::new().fg(lc)),
        ]));
    }
    list_lines.push(Line::from(""));
    list_lines.push(Line::from(Span::styled("←/→ to select", Style::new().fg(Color::DarkGray))));

    let list = Paragraph::new(list_lines)
        .block(Block::default().borders(Borders::RIGHT).style(Style::new().fg(Color::DarkGray)));
    frame.render_widget(list, left);

    let mut card_lines = Vec::new();
    if let Some(node) = app.nodes.get(sel) {
        let lc = if node.latency_ms <= 0.0 {
            Color::DarkGray
        } else if node.latency_ms < 2.0 {
            Color::Green
        } else if node.latency_ms < 8.0 {
            Color::Yellow
        } else {
            Color::Red
        };
        let (px, py) = project(node, app.camera.look_at_x(&app.nodes), app.camera.look_at_y(&app.nodes), app.camera.scale());

        card_lines.push(Line::from(Span::styled("  INFO CARD", Style::new().fg(Color::Cyan).bold())));
        card_lines.push(Line::from(""));
        card_lines.push(Line::from(vec![
            Span::styled("  Name:    ", Style::new().fg(Color::DarkGray)),
            Span::styled(&node.name, Style::new().fg(Color::White).bold()),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  Type:    ", Style::new().fg(Color::DarkGray)),
            Span::styled(&node.kind, Style::new().fg(node.color())),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  IP:      ", Style::new().fg(Color::DarkGray)),
            Span::styled(&node.ip, Style::new().fg(Color::White)),
        ]));

        if node.latency_ms <= 0.0 {
            card_lines.push(Line::from(vec![
                Span::styled("  Status:  ", Style::new().fg(Color::DarkGray)),
                Span::styled("offline", Style::new().fg(Color::DarkGray)),
            ]));
        } else {
            let tag = if node.latency_ms < 2.0 { "excellent" } else if node.latency_ms < 8.0 { "moderate" } else { "degraded" };
            card_lines.push(Line::from(vec![
                Span::styled("  Latency: ", Style::new().fg(Color::DarkGray)),
                Span::styled(format!("{:.1}ms", node.latency_ms), Style::new().fg(lc).bold()),
                Span::raw("  "),
                Span::styled(format!("({})", tag), Style::new().fg(lc)),
            ]));
            let spark = node.sparkline();
            if !spark.is_empty() {
                card_lines.push(Line::from(vec![
                    Span::styled("  History: ", Style::new().fg(Color::DarkGray)),
                    Span::styled(spark, Style::new().fg(Color::Cyan)),
                ]));
            }
        }

        card_lines.push(Line::from(vec![
            Span::styled("  3D pos:  (", Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{:.1}", node.x), Style::new().fg(Color::Blue)),
            Span::styled(", ", Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{:.1}", node.y), Style::new().fg(Color::Blue)),
            Span::styled(", ", Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{:.1}", node.z), Style::new().fg(Color::Magenta)),
            Span::styled(")", Style::new().fg(Color::DarkGray)),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  2D proj: (", Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{:.1}", px), Style::new().fg(Color::Green)),
            Span::styled(", ", Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{:.1}", py), Style::new().fg(Color::Green)),
            Span::styled(")", Style::new().fg(Color::DarkGray)),
        ]));

        let connected_to: Vec<&str> = neighbors_of(sel).iter()
            .filter_map(|&idx| app.nodes.get(idx).map(|n| n.name.as_str()))
            .collect();
        if !connected_to.is_empty() {
            card_lines.push(Line::from(""));
            card_lines.push(Line::from(vec![
                Span::styled("  Links:   ", Style::new().fg(Color::DarkGray)),
                Span::styled(connected_to.join(", "), Style::new().fg(Color::Cyan)),
            ]));
        }
        card_lines.push(Line::from(""));
        card_lines.push(Line::from(vec![
            Span::styled("  Scans:   ", Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{}", app.scan_count), Style::new().fg(Color::Yellow)),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  Updated: ", Style::new().fg(Color::DarkGray)),
            Span::styled(&app.last_update, Style::new().fg(Color::Green)),
        ]));
    }
    card_lines.push(Line::from(""));
    card_lines.push(Line::from(Span::styled("  Enter to restart", Style::new().fg(Color::DarkGray))));

    frame.render_widget(Paragraph::new(card_lines), right);
}
