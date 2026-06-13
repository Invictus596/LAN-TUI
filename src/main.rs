use std::f64::consts::PI;
use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Context, Line as CanvasLine};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;

const ZOOM_DURATION: f64 = 60.0;
const INTRO_DURATION: f64 = 90.0;

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

#[derive(Clone, Debug)]
struct Node {
    name: &'static str,
    ip: &'static str,
    kind: &'static str,
    latency_ms: f64,
    x: f64,
    y: f64,
    z: f64,
}

impl Node {
    fn mock_nodes() -> Vec<Self> {
        vec![
            Node { name: "Router", ip: "192.168.1.1", kind: "Gateway", latency_ms: 1.2, x: 0.0, y: 0.0, z: 0.0 },
            Node { name: "Laptop", ip: "192.168.1.42", kind: "Client", latency_ms: 3.7, x: 5.0, y: 3.0, z: 1.0 },
            Node { name: "Phone", ip: "192.168.1.67", kind: "Client", latency_ms: 5.1, x: -4.0, y: 4.5, z: 0.5 },
            Node { name: "Printer", ip: "192.168.1.15", kind: "Peripheral", latency_ms: 8.4, x: 3.5, y: -5.0, z: 1.2 },
        ]
    }

    fn color(&self) -> Color {
        match self.kind {
            "Gateway" => Color::Magenta,
            "Client" => Color::Cyan,
            "Peripheral" => Color::Yellow,
            _ => Color::White,
        }
    }
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

fn simulate_scan(current: &[Node], tick: u64) -> Vec<Node> {
    current
        .iter()
        .map(|n| {
            let jitter = ((tick.wrapping_mul(7).wrapping_add(n.name.len() as u64 * 13)) % 30) as f64
                * 0.3;
            let base = n.latency_ms;
            let delta = jitter - 4.5;
            let next = (base + delta).clamp(0.5, 99.9);
            Node {
                latency_ms: (next * 10.0).round() / 10.0,
                ..n.clone()
            }
        })
        .collect()
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
    let dx = node.x - look_at_x;
    let dy = node.y - look_at_y;
    (dx * scale, dy * scale)
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
}

impl App {
    fn new() -> Self {
        Self {
            state: StateMachine::Splash,
            nodes: Node::mock_nodes(),
            last_update: String::from("—"),
            scan_count: 0,
            camera: Camera::new(),
            selected_idx: 0,
            pulse_phase: 0.0,
            intro_progress: 0.0,
        }
    }

    fn advance(&mut self) {
        match self.state {
            StateMachine::Graph => {
                let next = (self.camera.target_idx + 1) % self.nodes.len();
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
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tx, rx) = mpsc::channel();
    let initial_nodes = Node::mock_nodes();
    thread::spawn(move || {
        let mut nodes = initial_nodes;
        let mut tick = 0u64;
        loop {
            thread::sleep(Duration::from_secs(2 + (tick % 2)));
            tick += 1;
            nodes = simulate_scan(&nodes, tick);
            if tx.send(nodes.clone()).is_err() {
                break;
            }
        }
    });

    let mut app = App::new();
    let mut running = true;

    while running {
        if let Ok(fresh) = rx.try_recv() {
            app.nodes = fresh;
            app.scan_count += 1;
            app.last_update = format!("last scan: {}", humantime_since_epoch());
        }
        app.update();
        terminal.draw(|f| ui(f, &app))?;
        running = handle_events(&mut app)?;
    }

    disable_raw_mode()?;
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
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Enter => app.advance(),
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(false),
                    KeyCode::Left => {
                        app.selected_idx = if app.selected_idx == 0 {
                            app.nodes.len() - 1
                        } else {
                            app.selected_idx - 1
                        };
                    }
                    KeyCode::Right => {
                        app.selected_idx = (app.selected_idx + 1) % app.nodes.len();
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(true)
}

fn ui(frame: &mut ratatui::Frame, app: &App) {
    let area = frame.area();
    render_status_bar(frame, area, app);
    render_main_content(frame, area, app);
}

fn render_status_bar(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let layout = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]);
    let [main_area, status_area] = layout.areas(area);

    let avg: f64 = if app.nodes.is_empty() {
        0.0
    } else {
        app.nodes.iter().map(|n| n.latency_ms).sum::<f64>() / app.nodes.len() as f64
    };

    let status = Line::from(vec![
        Span::styled(
            format!(" STATE: {} ", app.state.label()),
            Style::new().fg(Color::White).bg(Color::DarkGray),
        ),
        Span::raw(" — "),
        Span::styled(&app.last_update, Style::new().fg(Color::Green)),
        Span::raw("  "),
        Span::styled(format!("scans: {}", app.scan_count), Style::new().fg(Color::Blue)),
        Span::raw("  "),
        Span::styled(format!("avg: {:.1}ms", avg), Style::new().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled("←/→", Style::new().fg(Color::Cyan).bold()),
        Span::raw(" select · "),
        Span::styled("Enter", Style::new().fg(Color::Cyan).bold()),
        Span::raw(" next · "),
        Span::styled("q", Style::new().fg(Color::Cyan).bold()),
        Span::raw(" / "),
        Span::styled("Esc", Style::new().fg(Color::Cyan).bold()),
        Span::raw(" quit"),
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
    let text = vec![
        Line::from(Span::styled("LAN-TUI", Style::new().fg(Color::Magenta).bold().underlined())),
        Line::from(""),
        Line::from("A terminal-based LAN visualizer"),
        Line::from(""),
        Line::from(Span::styled("Press Enter to begin", Style::new().fg(Color::Cyan))),
    ];
    frame.render_widget(
        Paragraph::new(text).alignment(Alignment::Center).wrap(Wrap { trim: false }),
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
                x1: -extent,
                y1: scan_y,
                x2: extent,
                y2: scan_y,
                color: Color::Green,
            });

            for (i, node) in app.nodes.iter().enumerate() {
                let reveal = ((t * 120.0 - i as f64 * 20.0) / 20.0).clamp(0.0, 1.0);
                if reveal <= 0.0 {
                    continue;
                }
                let alpha = if reveal < 1.0 {
                    (reveal * 3.0).min(1.0)
                } else {
                    1.0
                };
                let past_scan = scan_y <= node.y;

                if past_scan || reveal > 0.0 {
                    let c = if alpha < 0.5 {
                        Color::DarkGray
                    } else {
                        node.color()
                    };
                    draw_node_dot(ctx, node.x, node.y, c);
                    ctx.print(node.x, node.y + 0.6, format!("{}", node.name));
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
        )))
        .alignment(Alignment::Center)
    } else {
        let bar_width: usize = 20;
        let filled = (t * bar_width as f64).round() as usize;
        let remainder = bar_width.saturating_sub(filled);
        let bar: String = format!(
            "[{}{}]",
            "■".repeat(filled),
            "·".repeat(remainder)
        );
        Paragraph::new(Line::from(Span::styled(bar, Style::new().fg(Color::Cyan))))
            .alignment(Alignment::Center)
    };
    frame.render_widget(
        msg,
        Rect::new(area.x, area.bottom().saturating_sub(3), area.width, 1),
    );
}

fn draw_edges(
    ctx: &mut Context<'_>,
    nodes: &[Node],
    selected: usize,
    highlight: bool,
) {
    for (i, j) in &edge_indices() {
        if let (Some(a), Some(b)) = (nodes.get(*i), nodes.get(*j)) {
            let is_connected = *i == selected || *j == selected;
            let color = if highlight && is_connected {
                Color::Cyan
            } else {
                Color::DarkGray
            };
            ctx.draw(&CanvasLine {
                x1: a.x,
                y1: a.y,
                x2: b.x,
                y2: b.y,
                color,
            });
            if is_connected || !highlight {
                let mx = (a.x + b.x) / 2.0;
                let my = (a.y + b.y) / 2.0;
                let avg_lat = (a.latency_ms + b.latency_ms) / 2.0;
                ctx.print(mx, my, format!("{:.1}ms", avg_lat));
            }
        }
    }
}

fn render_graph_canvas(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let scale = app.camera.scale();
    let look_at_x = app.camera.look_at_x(&app.nodes);
    let look_at_y = app.camera.look_at_y(&app.nodes);
    let extent = 12.0 / scale;
    let sel = app.selected_idx;
    let pulse = app.pulse_phase.sin() * 0.3 + 0.7;

    let canvas = Canvas::default()
        .x_bounds([-extent + look_at_x, extent + look_at_x])
        .y_bounds([-extent + look_at_y, extent + look_at_y])
        .paint(|ctx| {
            draw_edges(ctx, &app.nodes, sel, true);
            for (i, node) in app.nodes.iter().enumerate() {
                let is_selected = i == sel;
                let base_color = node.color();
                let dot_color = if is_selected {
                    Color::LightCyan
                } else if pulse > 0.85 && i != 0 {
                    Color::LightBlue
                } else {
                    base_color
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
                    ctx.print(
                        node.x,
                        node.y + 0.6,
                        format!("{}  {}", node.ip, node.kind),
                    );
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
    let sel = app.selected_idx;

    let canvas = Canvas::default()
        .x_bounds([-extent + look_at_x, extent + look_at_x])
        .y_bounds([-extent + look_at_y, extent + look_at_y])
        .paint(|ctx| {
            draw_edges(ctx, &app.nodes, sel, false);
            for (_, node) in app.nodes.iter().enumerate() {
                let dx = node.x - look_at_x;
                let dy = node.y - look_at_y;
                let dist = (dx * dx + dy * dy).sqrt();
                let fade = (1.0 - (dist / 8.0).clamp(0.0, 1.0)).max(0.1);
                if fade > 0.1 {
                    draw_node_dot(ctx, node.x, node.y, node.color());
                }
                ctx.print(node.x, node.y - 0.6, format!("{}", node.name));
                ctx.print(node.x, node.y + 0.4, format!("{:.1}ms", node.latency_ms));
            }
        });

    frame.render_widget(canvas, area);
    let overlay = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("ZOOMING → {}", app.nodes[app.camera.target_idx].name),
            Style::new().fg(Color::Yellow).bold(),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:.0}%", app.camera.zoom_progress * 100.0),
            Style::new().fg(Color::Cyan),
        ),
    ]))
    .alignment(Alignment::Center);
    let top = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(overlay, top);
}

fn render_detail_panel(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let chunks = Layout::horizontal([
        Constraint::Length(30),
        Constraint::Min(10),
    ])
    .split(area);
    let [left, right] = [chunks[0], chunks[1]];

    let mut list_lines = vec![Line::from(
        Span::styled("NODES", Style::new().fg(Color::Cyan).bold()),
    )];
    list_lines.push(Line::from(
        Span::styled("───".repeat(10), Style::new().fg(Color::DarkGray)),
    ));
    for (i, node) in app.nodes.iter().enumerate() {
        let marker = if i == app.selected_idx { "▶" } else { " " };
        let c = if i == app.selected_idx {
            Color::LightCyan
        } else {
            Color::White
        };
        let latency_color = if node.latency_ms < 2.0 {
            Color::Green
        } else if node.latency_ms < 8.0 {
            Color::Yellow
        } else {
            Color::Red
        };
        list_lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(Color::Yellow).bold()),
            Span::raw(" "),
            Span::styled(node.name, Style::new().fg(c).bold()),
            Span::raw("  "),
            Span::styled(
                format!("{:.1}ms", node.latency_ms),
                Style::new().fg(latency_color),
            ),
        ]));
    }
    list_lines.push(Line::from(""));
    list_lines.push(Line::from(Span::styled(
        "←/→ to select",
        Style::new().fg(Color::DarkGray),
    )));

    let list = Paragraph::new(list_lines).block(
        Block::default()
            .borders(Borders::RIGHT)
            .style(Style::new().fg(Color::DarkGray)),
    );
    frame.render_widget(list, left);

    let mut card_lines = Vec::new();
    if let Some(node) = app.nodes.get(app.selected_idx) {
        let latency_color = if node.latency_ms < 2.0 {
            Color::Green
        } else if node.latency_ms < 8.0 {
            Color::Yellow
        } else {
            Color::Red
        };
        let (px, py) = project(
            node,
            app.camera.look_at_x(&app.nodes),
            app.camera.look_at_y(&app.nodes),
            app.camera.scale(),
        );

        card_lines.push(Line::from(Span::styled(
            "  INFO CARD",
            Style::new().fg(Color::Cyan).bold(),
        )));
        card_lines.push(Line::from(""));
        card_lines.push(Line::from(vec![
            Span::styled("  Name:    ", Style::new().fg(Color::DarkGray)),
            Span::styled(node.name, Style::new().fg(Color::White).bold()),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  Type:    ", Style::new().fg(Color::DarkGray)),
            Span::styled(node.kind, Style::new().fg(node.color())),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  IP:      ", Style::new().fg(Color::DarkGray)),
            Span::styled(node.ip, Style::new().fg(Color::White)),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  Latency: ", Style::new().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1}ms", node.latency_ms),
                Style::new().fg(latency_color).bold(),
            ),
            Span::raw("  "),
            Span::styled(
                match node.latency_ms {
                    x if x < 2.0 => "(excellent)",
                    x if x < 8.0 => "(moderate)",
                    _ => "(degraded)",
                },
                Style::new().fg(latency_color),
            ),
        ]));
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
        card_lines.push(Line::from(""));

        let connected_to: Vec<&str> = neighbors_of(app.selected_idx)
            .iter()
            .filter_map(|&idx| app.nodes.get(idx).map(|n| n.name))
            .collect();
        if !connected_to.is_empty() {
            card_lines.push(Line::from(vec![
                Span::styled("  Links:   ", Style::new().fg(Color::DarkGray)),
                Span::styled(connected_to.join(", "), Style::new().fg(Color::Cyan)),
            ]));
        }
        card_lines.push(Line::from(""));
        card_lines.push(Line::from(vec![
            Span::styled("  Scans:   ", Style::new().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", app.scan_count),
                Style::new().fg(Color::Yellow),
            ),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  Updated: ", Style::new().fg(Color::DarkGray)),
            Span::styled(&app.last_update, Style::new().fg(Color::Green)),
        ]));
    }
    card_lines.push(Line::from(""));
    card_lines.push(Line::from(Span::styled(
        "  Enter to restart",
        Style::new().fg(Color::DarkGray),
    )));

    let card = Paragraph::new(card_lines);
    frame.render_widget(card, right);
}
