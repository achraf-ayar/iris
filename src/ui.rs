//! ratatui rendering. Left: active session list. Right: detail for the
//! selected session — header, token/cost stats, tool timeline, live feed.

use chrono::Local;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, SummaryState};
use crate::cost;
use crate::session::{Event, EventKind, Session, Status};

/// Icon, color, and short label for a session's current status.
pub fn status_glyph(s: &Session) -> (&'static str, Color, String) {
    match s.status() {
        Status::NeedsApproval => {
            let label = match &s.pending_tool {
                Some(t) => format!("NEEDS APPROVAL · {t}"),
                None => "NEEDS APPROVAL".into(),
            };
            ("⚠", Color::Red, label)
        }
        Status::Working => {
            let label = match &s.pending_tool {
                Some(t) => format!("running · {t}"),
                None => "working".into(),
            };
            ("●", Color::Green, label)
        }
        Status::Done => ("✓", Color::Blue, "done · awaiting you".into()),
        Status::Idle => ("○", Color::DarkGray, "idle".into()),
    }
}

pub fn short_model(model: Option<&str>) -> String {
    let m = model.unwrap_or("?");
    m.strip_prefix("claude-")
        .unwrap_or(m)
        .split('[')
        .next()
        .unwrap_or(m)
        .to_string()
}

pub fn human_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_header(f, app, root[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(root[1]);

    draw_list(f, app, cols[0]);
    draw_detail(f, app, cols[1]);
    draw_footer(f, root[2]);

    if app.popup_open {
        draw_summary_popup(f, app);
    }
    if app.approve_open {
        draw_approval_popup(f, app);
    }
    if app.editing_key {
        draw_key_popup(f, app);
    }
    if app.install_open {
        draw_install_popup(f, app);
    }
}

fn draw_install_popup(f: &mut Frame, app: &App) {
    let area = centered_rect(72, 46, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" approvals ")
        .border_style(Style::new().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let (body, hint) = if app.hook_installed {
        (
            vec![
                Line::from(Span::styled(
                    "Approval interception is ON.",
                    Style::new().fg(Color::Green).bold(),
                )),
                Line::from(""),
                Line::from(
                    "iris intercepts tool-approval prompts from your other Claude Code \
                     sessions via a PreToolUse hook in ~/.claude/settings.json.",
                ),
                Line::from(""),
                Line::from(Span::styled(
                    "Disable it (remove the hook)?",
                    Style::new().fg(Color::DarkGray),
                )),
            ],
            Line::from(vec![
                Span::styled(" x ", Style::new().fg(Color::Black).bg(Color::Red).bold()),
                Span::styled(" disable    ", Style::new().fg(Color::Red)),
                Span::styled(" Esc ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
                Span::styled(" keep enabled", Style::new().fg(Color::DarkGray)),
            ]),
        )
    } else {
        (
            vec![
                Line::from(Span::styled(
                    "Let iris approve tool calls from your other Claude sessions?",
                    Style::new().fg(Color::White).bold(),
                )),
                Line::from(""),
                Line::from(
                    "This adds a PreToolUse hook to ~/.claude/settings.json. While iris \
                     is running, each tool call from any session waits for your decision \
                     here (a allow / d deny); if iris isn't running it falls back to the \
                     normal prompt. You'll need to restart sessions to arm it.",
                ),
                Line::from(""),
                Line::from(Span::styled(
                    "Enable approval interception?",
                    Style::new().fg(Color::DarkGray),
                )),
            ],
            Line::from(vec![
                Span::styled(" a ", Style::new().fg(Color::Black).bg(Color::Green).bold()),
                Span::styled(" accept / enable    ", Style::new().fg(Color::Green)),
                Span::styled(" r ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
                Span::styled(" refuse", Style::new().fg(Color::DarkGray)),
            ]),
        )
    };
    f.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), rows[0]);
    f.render_widget(Paragraph::new(hint), rows[1]);
}

fn draw_approval_popup(f: &mut Frame, app: &App) {
    let p = match app.current_pending() {
        Some(p) => p,
        None => return,
    };
    let area = centered_rect(76, 70, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" approve — {} ", p.tool_name))
        .border_style(Style::new().fg(Color::Red));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // cwd
            Constraint::Length(5), // AI risk assessment
            Constraint::Min(0),    // full input
            Constraint::Length(1), // hints
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("dir ", Style::new().fg(Color::DarkGray)),
            Span::styled(p.cwd.clone(), Style::new().fg(Color::Cyan)),
        ])),
        rows[0],
    );

    let assess = match app.current_assessment() {
        Some(SummaryState::Done(t)) => {
            Paragraph::new(t.clone()).style(Style::new().fg(Color::White))
        }
        Some(SummaryState::Loading) => {
            Paragraph::new("assessing risk…").style(Style::new().fg(Color::Yellow))
        }
        Some(SummaryState::Error(e)) => {
            Paragraph::new(format!("risk check failed: {e}")).style(Style::new().fg(Color::Red))
        }
        None => Paragraph::new("press x for an AI risk read")
            .style(Style::new().fg(Color::DarkGray)),
    };
    let assess_block = Block::default()
        .borders(Borders::ALL)
        .title(" risk ")
        .border_style(Style::new().fg(Color::DarkGray));
    f.render_widget(assess.block(assess_block).wrap(Wrap { trim: false }), rows[1]);

    let input_block = Block::default()
        .borders(Borders::TOP)
        .title(" tool input ")
        .border_style(Style::new().fg(Color::DarkGray));
    f.render_widget(
        Paragraph::new(p.input.clone())
            .style(Style::new().fg(Color::Gray))
            .block(input_block)
            .wrap(Wrap { trim: false }),
        rows[2],
    );

    let hint = Line::from(vec![
        Span::styled(" a ", Style::new().fg(Color::Black).bg(Color::Green).bold()),
        Span::styled(" allow   ", Style::new().fg(Color::Green)),
        Span::styled(" d ", Style::new().fg(Color::Black).bg(Color::Red).bold()),
        Span::styled(" deny   ", Style::new().fg(Color::Red)),
        Span::styled(" x ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" risk   ", Style::new().fg(Color::DarkGray)),
        Span::styled(" Esc ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" close", Style::new().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hint), rows[3]);
}

fn draw_key_popup(f: &mut Frame, app: &App) {
    let area = centered_rect(64, 30, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" set ANTHROPIC API key ")
        .border_style(Style::new().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Mask the key — show a dot per character plus a cursor.
    let masked: String = "•".repeat(app.key_buffer.chars().count());
    let lines = vec![
        Line::from(Span::styled(
            "paste or type your key, then Enter:",
            Style::new().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            format!("{masked}▏"),
            Style::new().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Enter save · Esc cancel · stored 0600 in ~/.claude/iris/api_key",
            Style::new().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_summary_popup(f: &mut Frame, app: &App) {
    let area = centered_rect(70, 60, f.area());
    f.render_widget(Clear, area);

    let title = match app.selected_session() {
        Some(s) => format!(" summary · {} ", s.project()),
        None => " summary ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::new().fg(Color::Magenta));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let body: Paragraph = match app.selected_summary() {
        Some(SummaryState::Done(text)) => {
            Paragraph::new(text.clone()).style(Style::new().fg(Color::White))
        }
        Some(SummaryState::Loading) => Paragraph::new(format!(
            "generating summary with {}…",
            short_model(Some(crate::anthropic::SUMMARY_MODEL))
        ))
        .style(Style::new().fg(Color::Yellow)),
        Some(SummaryState::Error(e)) => {
            Paragraph::new(format!("error: {e}")).style(Style::new().fg(Color::Red))
        }
        None => Paragraph::new("no session selected").style(Style::new().fg(Color::DarkGray)),
    };
    f.render_widget(body.wrap(Wrap { trim: false }), rows[0]);

    let hint = Line::from(vec![
        Span::styled(" g ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" regenerate   ", Style::new().fg(Color::DarkGray)),
        Span::styled(" Esc ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" close", Style::new().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hint), rows[1]);
}

/// A rect centered in `area`, sized as a percentage of width/height.
fn centered_rect(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let now = Local::now().format("%H:%M:%S").to_string();
    let count = app.visible.len();
    let npending = app.pending.len();

    // Flash and warnings go right after the badge so they never clip off-screen.
    let mut spans = vec![Span::styled(
        " iris ",
        Style::new().fg(Color::Black).bg(Color::Magenta).bold(),
    )];

    if let Some(status) = &app.flash {
        spans.push(Span::styled(
            format!(" {status} "),
            Style::new().fg(Color::Black).bg(Color::Green).bold(),
        ));
    }

    spans.push(Span::styled(
        format!("  {count} active"),
        Style::new().fg(Color::White).bold(),
    ));

    // Pending approvals — red when there are any, so it's obvious a/d can act.
    let pend_style = if npending > 0 {
        Style::new().fg(Color::Black).bg(Color::Red).bold()
    } else {
        Style::new().fg(Color::DarkGray)
    };
    spans.push(Span::styled(format!("  pending {npending} "), pend_style));

    if !app.hook_installed {
        spans.push(Span::styled(
            "  ⚑ approvals off — press i to enable",
            Style::new().fg(Color::Yellow).bold(),
        ));
    } else if !app.has_api_key() {
        spans.push(Span::styled(
            "  · no API key (press K)",
            Style::new().fg(Color::Yellow),
        ));
    }

    spans.push(Span::styled(
        format!("  · {}m · {now}", app.window.as_secs() / 60),
        Style::new().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" sessions ")
        .border_style(Style::new().fg(Color::DarkGray));

    if app.visible.is_empty() {
        let p = Paragraph::new("no active sessions in window")
            .style(Style::new().fg(Color::DarkGray))
            .block(block);
        f.render_widget(p, area);
        return;
    }

    let items: Vec<ListItem> = app
        .sessions()
        .map(|s| {
            // A live hook approval overrides the transcript-derived status.
            let (icon, color, state) = match app.pending.get(&s.id) {
                Some(p) => ("⚠", Color::Red, format!("APPROVE {} — a/d", p.tool_name)),
                None => status_glyph(s),
            };
            let header = Line::from(vec![
                Span::styled(icon, Style::new().fg(color).bold()),
                Span::raw(" "),
                Span::styled(s.label(), Style::new().fg(Color::White)),
            ]);
            let meta = Line::from(vec![
                Span::styled(format!("  {state}"), Style::new().fg(color).bold()),
                Span::styled(format!(" · {}", s.project()), Style::new().fg(Color::Cyan)),
                Span::styled(
                    format!(" · {}", short_model(s.model.as_deref())),
                    Style::new().fg(Color::Magenta),
                ),
                Span::styled(
                    format!(" · {} tok", human_tokens(s.usage.total())),
                    Style::new().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(vec![header, meta])
        })
        .collect();

    let list = List::new(items).block(block).highlight_style(
        Style::new()
            .bg(Color::Rgb(40, 40, 55))
            .add_modifier(Modifier::BOLD),
    );

    let mut state = ListState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" detail ")
        .border_style(Style::new().fg(Color::DarkGray));

    let s = match app.selected_session() {
        Some(s) => s,
        None => {
            f.render_widget(block, area);
            return;
        }
    };

    let inner = block.inner(area);
    f.render_widget(block, area);

    let pending = app.pending.get(&s.id);
    let banner_rows = if pending.is_some() { 4 } else { 0 };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(banner_rows), // pending approval banner
            Constraint::Length(2),           // header (title + path)
            Constraint::Length(2),           // stats
            Constraint::Length(tool_rows(s)),
            Constraint::Min(0), // feed
        ])
        .split(inner);

    if let Some(p) = pending {
        let banner = vec![
            Line::from(Span::styled(
                {
                    let dir = p.cwd.rsplit('/').next().unwrap_or(&p.cwd);
                    format!("⚠ PENDING APPROVAL — {} in {dir}", p.tool_name)
                },
                Style::new().fg(Color::Red).bold(),
            )),
            Line::from(Span::styled(p.brief.clone(), Style::new().fg(Color::White))),
            Line::from(vec![
                Span::styled(" a ", Style::new().fg(Color::Black).bg(Color::Green).bold()),
                Span::styled(" allow    ", Style::new().fg(Color::Green)),
                Span::styled(" d ", Style::new().fg(Color::Black).bg(Color::Red).bold()),
                Span::styled(" deny", Style::new().fg(Color::Red)),
            ]),
        ];
        f.render_widget(Paragraph::new(banner).wrap(Wrap { trim: false }), rows[0]);
    }

    // header
    let branch = s
        .branch
        .as_deref()
        .map(|b| format!("  @{b}"))
        .unwrap_or_default();
    let (icon, scolor, state) = status_glyph(s);
    let head = vec![
        Line::from(vec![
            Span::styled(format!("{icon} "), Style::new().fg(scolor).bold()),
            Span::styled(s.label(), Style::new().fg(Color::White).bold()),
            Span::styled(format!("   [{state}]"), Style::new().fg(scolor).bold()),
        ]),
        Line::from(vec![
            Span::styled(
                s.cwd.clone().unwrap_or_else(|| "?".into()),
                Style::new().fg(Color::Cyan),
            ),
            Span::styled(branch, Style::new().fg(Color::Green)),
        ]),
    ];
    f.render_widget(Paragraph::new(head), rows[1]);

    // stats
    let u = &s.usage;
    let est = cost::estimate(u, s.model.as_deref());
    let stats = vec![
        Line::from(vec![
            Span::styled("model ", Style::new().fg(Color::DarkGray)),
            Span::styled(short_model(s.model.as_deref()), Style::new().fg(Color::Magenta)),
            Span::styled("   turns ", Style::new().fg(Color::DarkGray)),
            Span::styled(s.assistant_turns.to_string(), Style::new().fg(Color::White)),
            Span::styled("   ~cost ", Style::new().fg(Color::DarkGray)),
            Span::styled(format!("${est:.2}"), Style::new().fg(Color::Green).bold()),
        ]),
        Line::from(vec![
            Span::styled("in ", Style::new().fg(Color::DarkGray)),
            Span::raw(human_tokens(u.input)),
            Span::styled("  out ", Style::new().fg(Color::DarkGray)),
            Span::raw(human_tokens(u.output)),
            Span::styled("  cache w/r ", Style::new().fg(Color::DarkGray)),
            Span::raw(format!(
                "{}/{}",
                human_tokens(u.cache_creation),
                human_tokens(u.cache_read)
            )),
        ]),
    ];
    f.render_widget(Paragraph::new(stats), rows[2]);

    draw_tools(f, s, rows[3]);
    draw_feed(f, s, rows[4]);
}

/// One row for a "tools" heading plus up to a few tool bars.
fn tool_rows(s: &Session) -> u16 {
    if s.tool_counts.is_empty() {
        0
    } else {
        1 + s.tool_counts.len().min(6) as u16
    }
}

fn draw_tools(f: &mut Frame, s: &Session, area: Rect) {
    if s.tool_counts.is_empty() || area.height == 0 {
        return;
    }
    let max = s.tool_counts.values().copied().max().unwrap_or(1).max(1);
    let mut pairs: Vec<(&String, &u64)> = s.tool_counts.iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(a.1));

    let mut lines = vec![Line::from(Span::styled(
        "tool calls",
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::UNDERLINED),
    ))];
    for (name, n) in pairs.into_iter().take(6) {
        let width = 18usize;
        let filled = ((*n as f64 / max as f64) * width as f64).round() as usize;
        let bar: String = "█".repeat(filled) + &"░".repeat(width - filled);
        lines.push(Line::from(vec![
            Span::styled(format!("{name:<14}"), Style::new().fg(Color::Yellow)),
            Span::styled(bar, Style::new().fg(Color::Yellow)),
            Span::styled(format!(" {n}"), Style::new().fg(Color::White)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_feed(f: &mut Frame, s: &Session, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .title(" activity ")
        .border_style(Style::new().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let cap = inner.height as usize;
    if cap == 0 {
        return;
    }
    let start = s.events.len().saturating_sub(cap);
    let lines: Vec<Line> = s
        .events
        .iter()
        .skip(start)
        .map(event_line)
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn event_line(e: &Event) -> Line<'static> {
    let ts = e
        .ts
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".into());
    let (icon, color, label): (&str, Color, String) = match &e.kind {
        EventKind::Prompt => ("▸", Color::Cyan, "you".into()),
        EventKind::Assistant => ("✷", Color::White, "claude".into()),
        EventKind::Thinking => ("·", Color::DarkGray, "think".into()),
        EventKind::Tool(name) => ("⚒", Color::Yellow, name.clone()),
        EventKind::ToolResult { error } => {
            if *error {
                ("✗", Color::Red, "result".into())
            } else {
                ("←", Color::Green, "result".into())
            }
        }
    };
    Line::from(vec![
        Span::styled(format!("{ts} "), Style::new().fg(Color::DarkGray)),
        Span::styled(format!("{icon} "), Style::new().fg(color)),
        Span::styled(format!("{label} ", ), Style::new().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(e.text.clone(), Style::new().fg(Color::Gray)),
    ])
}

fn draw_footer(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" j/k ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" move  ", Style::new().fg(Color::DarkGray)),
        Span::styled(" a/d ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" allow/deny  ", Style::new().fg(Color::DarkGray)),
        Span::styled(" ⏎ ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" approve details  ", Style::new().fg(Color::DarkGray)),
        Span::styled(" s ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" summary  ", Style::new().fg(Color::DarkGray)),
        Span::styled(" i ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" approvals  ", Style::new().fg(Color::DarkGray)),
        Span::styled(" K ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" api key  ", Style::new().fg(Color::DarkGray)),
        Span::styled(" r ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" refresh  ", Style::new().fg(Color::DarkGray)),
        Span::styled(" q ", Style::new().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled(" quit", Style::new().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
