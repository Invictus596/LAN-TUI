use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;

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
}

impl Node {
    fn mock_nodes() -> Vec<Self> {
        vec![
            Node { name: "Router", ip: "192.168.1.1", kind: "Gateway" },
            Node { name: "Laptop", ip: "192.168.1.42", kind: "Client" },
            Node { name: "Phone", ip: "192.168.1.67", kind: "Client" },
            Node { name: "Printer", ip: "192.168.1.15", kind: "Peripheral" },
        ]
    }
}

struct App {
    state: StateMachine,
    nodes: Vec<Node>,
}

impl App {
    fn new() -> Self {
        Self {
            state: StateMachine::Splash,
            nodes: Node::mock_nodes(),
        }
    }

    fn advance(&mut self) {
        self.state = self.state.transition();
    }
}

fn main() -> io::Result<()> {
    color_eyre::install().ok();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new();
    let mut result: io::Result<()> = Ok(());

    while result.is_ok() {
        terminal.draw(|f| ui(f, &app))?;
        result = handle_events(&mut app);
    }

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    result
}

fn handle_events(app: &mut App) -> io::Result<()> {
    if let Event::Key(key) = event::read()? {
        if key.kind == KeyEventKind::Press {
            match key.code {
                KeyCode::Enter => app.advance(),
                KeyCode::Char('q') | KeyCode::Esc => {
                    return Ok(());
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn ui(frame: &mut ratatui::Frame, app: &App) {
    let area = frame.area();
    render_status_bar(frame, area, app.state);
    render_main_content(frame, area, app);
}

fn render_status_bar(frame: &mut ratatui::Frame, area: Rect, state: StateMachine) {
    let layout = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]);
    let [main_area, status_area] = layout.areas(area);

    let status = Line::from(vec![
        Span::styled(
            format!(" STATE: {} ", state.label()),
            Style::new().fg(Color::White).bg(Color::DarkGray),
        ),
        Span::raw(" — "),
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
        StateMachine::AnimatingIntro => render_animating_intro(frame, content_area),
        StateMachine::Graph => render_graph(frame, content_area, &app.nodes),
        StateMachine::AnimatingZoom => render_animating_zoom(frame, content_area, &app.nodes),
        StateMachine::Detail => render_detail(frame, content_area, &app.nodes),
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
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_animating_intro(frame: &mut ratatui::Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled("ANIMATING INTRO", Style::new().fg(Color::Yellow).bold())),
        Line::from(""),
        Line::from("[ Scanning network... ]"),
        Line::from(""),
        Line::from(Span::styled("(Placeholder — Enter to continue)", Style::new().fg(Color::DarkGray))),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_graph(frame: &mut ratatui::Frame, area: Rect, nodes: &[Node]) {
    let lines: Vec<Line> = std::iter::once(Line::from(
        Span::styled("NETWORK GRAPH", Style::new().fg(Color::Green).bold()),
    ))
    .chain(std::iter::once(Line::from("")))
    .chain(nodes.iter().map(|node| {
        Line::from(vec![
            Span::styled(format!("◉  {} ", node.name), Style::new().fg(Color::Cyan).bold()),
            Span::styled(node.ip, Style::new().fg(Color::White)),
            Span::raw("  "),
            Span::styled(format!("[{}]", node.kind), Style::new().fg(Color::DarkGray)),
        ])
    }))
    .chain(std::iter::once(Line::from("")))
    .chain(std::iter::once(Line::from(
        Span::styled("(Placeholder — Enter to continue)", Style::new().fg(Color::DarkGray)),
    )))
    .collect();

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_animating_zoom(frame: &mut ratatui::Frame, area: Rect, nodes: &[Node]) {
    let lines: Vec<Line> = std::iter::once(Line::from(
        Span::styled("ANIMATING ZOOM", Style::new().fg(Color::Yellow).bold()),
    ))
    .chain(std::iter::once(Line::from("")))
    .chain(nodes.iter().map(|node| {
        Line::from(Span::styled(
            format!("  ~ zooming to {} ...", node.name),
            Style::new().fg(Color::Blue),
        ))
    }))
    .chain(std::iter::once(Line::from("")))
    .chain(std::iter::once(Line::from(
        Span::styled("(Placeholder — Enter to continue)", Style::new().fg(Color::DarkGray)),
    )))
    .collect();

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_detail(frame: &mut ratatui::Frame, area: Rect, nodes: &[Node]) {
    let lines: Vec<Line> = std::iter::once(Line::from(
        Span::styled("NODE DETAIL", Style::new().fg(Color::Cyan).bold()),
    ))
    .chain(std::iter::once(Line::from("")))
    .chain(nodes.iter().map(|node| {
        Line::from(vec![
            Span::styled("┌─ ", Style::new().fg(Color::DarkGray)),
            Span::styled(node.name, Style::new().fg(Color::Cyan).bold()),
        ])
    }))
    .chain(std::iter::once(Line::from("")))
    .chain(std::iter::once(Line::from(
        Span::styled("(Placeholder — Enter to restart)", Style::new().fg(Color::DarkGray)),
    )))
    .collect();

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}
