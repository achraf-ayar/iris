//! iris — a live supervisor for all active Claude Code sessions.
//!
//! Reads the transcript files Claude Code writes under
//! `~/.claude/projects/<slug>/<uuid>.jsonl`, tails the active ones, and renders
//! a dashboard of what every running session is currently doing.

mod anthropic;
mod app;
mod bridge;
mod cost;
mod session;
mod ui;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use app::App;

struct Args {
    window: Duration,
    interval: Duration,
    dir: Option<PathBuf>,
    list_once: bool,
    hook: bool,
    install_hook: bool,
    uninstall_hook: bool,
    project: bool,
}

const HELP: &str = "\
iris — supervise all active Claude Code sessions

USAGE:
    iris [OPTIONS] [COMMAND]

COMMANDS:
    ls            Print a one-shot table of active sessions and exit
    install-hook  Register `iris hook` as a PreToolUse hook in settings.json
                  so approvals route through iris. --project for ./.claude.
    uninstall-hook  Remove the iris PreToolUse hook from settings.json.
    hook          PreToolUse hook bridge — reads a tool request on stdin and
                  routes the approve/deny decision through the iris TUI.
                  Register it in settings.json, not run by hand.
    (default)     Launch the live TUI dashboard

OPTIONS:
    -w, --window <MIN>      Treat a session active if touched within MIN minutes [default: 3]
                            (sessions waiting for approval stay shown regardless)
    -i, --interval <SEC>    Data refresh interval in seconds [default: 1]
    -d, --dir <PATH>        Override the projects directory [default: ~/.claude/projects]
    -h, --help              Show this help
";

fn parse_args() -> Result<Args> {
    let mut a = Args {
        window: Duration::from_secs(3 * 60),
        interval: Duration::from_secs(1),
        dir: None,
        list_once: false,
        hook: false,
        install_hook: false,
        uninstall_hook: false,
        project: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "ls" | "list" => a.list_once = true,
            "hook" => a.hook = true,
            "install-hook" => a.install_hook = true,
            "uninstall-hook" => a.uninstall_hook = true,
            "--project" => a.project = true,
            "-w" | "--window" => {
                let v: u64 = it.next().context("--window needs a value")?.parse()?;
                a.window = Duration::from_secs(v * 60);
            }
            "-i" | "--interval" => {
                let v: u64 = it.next().context("--interval needs a value")?.parse()?;
                a.interval = Duration::from_secs(v.max(1));
            }
            "-d" | "--dir" => {
                a.dir = Some(PathBuf::from(it.next().context("--dir needs a value")?));
            }
            other => anyhow::bail!("unknown argument: {other}\n\n{HELP}"),
        }
    }
    Ok(a)
}

fn projects_dir(arg: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(d) = arg {
        return Ok(d);
    }
    let home = dirs::home_dir().context("cannot resolve home directory")?;
    Ok(home.join(".claude").join("projects"))
}

fn main() -> Result<()> {
    let mut args = parse_args()?;

    // The hook bridge runs as a Claude Code subprocess: read stdin, print the
    // decision, exit. It must not touch the projects dir or print anything else.
    if args.hook {
        std::process::exit(bridge::run_hook());
    }

    if args.install_hook {
        match bridge::install_hook(args.project) {
            Ok(msg) => {
                println!("{msg}");
                return Ok(());
            }
            Err(e) => anyhow::bail!("install-hook failed: {e}"),
        }
    }

    if args.uninstall_hook {
        match bridge::uninstall_hook(args.project) {
            Ok(msg) => {
                println!("{msg}");
                return Ok(());
            }
            Err(e) => anyhow::bail!("uninstall-hook failed: {e}"),
        }
    }

    let dir = projects_dir(args.dir.take())?;
    if !dir.exists() {
        anyhow::bail!("projects directory not found: {}", dir.display());
    }

    let app = App::new(dir, args.window, args.interval);

    if args.list_once {
        print_list(&app);
        return Ok(());
    }

    run_tui(app)
}

fn print_list(app: &App) {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if app.visible.is_empty() {
        let _ = writeln!(out, "no active sessions");
        return;
    }
    // Ignore write errors (e.g. a broken pipe when piped into `head`).
    let _ = writeln!(
        out,
        "{:<3} {:<22} {:<16} {:<13} {:>8} {:>7}  {}",
        "", "STATE", "PROJECT", "MODEL", "TOKENS", "~COST", "TITLE"
    );
    for s in app.sessions() {
        let (icon, _color, state) = ui::status_glyph(s);
        let est = cost::estimate(&s.usage, s.model.as_deref());
        if writeln!(
            out,
            "{:<3} {:<22} {:<16} {:<13} {:>8} {:>7.2}  {}",
            icon,
            truncate(&state, 22),
            truncate(s.project(), 16),
            truncate(&ui::short_model(s.model.as_deref()), 13),
            ui::human_tokens(s.usage.total()),
            est,
            s.label(),
        )
        .is_err()
        {
            break;
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).chain(['…']).collect()
    }
}

fn run_tui(mut app: App) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let poll = Duration::from_millis(150);
    loop {
        app.tick();
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(poll)? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                if app.editing_key {
                    // Modal text entry for the API key.
                    match k.code {
                        KeyCode::Enter => app.commit_key_input(),
                        KeyCode::Esc => app.cancel_key_input(),
                        KeyCode::Backspace => app.key_input_backspace(),
                        KeyCode::Char(c) => app.key_input_push(c),
                        _ => {}
                    }
                } else if app.install_open {
                    // Proposal: enable/disable approval interception.
                    match k.code {
                        KeyCode::Char('a') | KeyCode::Char('y') | KeyCode::Enter
                            if !app.hook_installed =>
                        {
                            app.enable_approvals()
                        }
                        KeyCode::Char('u') | KeyCode::Char('x') if app.hook_installed => {
                            app.disable_approvals()
                        }
                        KeyCode::Char('r') | KeyCode::Char('n') | KeyCode::Esc => {
                            app.close_install()
                        }
                        KeyCode::Char('q') => break,
                        _ => {}
                    }
                } else if app.approve_open {
                    // Approval modal: a/d decide, x risk-checks, Esc closes.
                    match k.code {
                        KeyCode::Char('a') => app.approve_selected(true),
                        KeyCode::Char('d') => app.approve_selected(false),
                        KeyCode::Char('x') => app.assess_pending(),
                        KeyCode::Esc => app.close_approval(),
                        KeyCode::Char('q') => break,
                        _ => {}
                    }
                } else if app.popup_open {
                    // Popup-mode keys: Esc/s close, g regenerates, q quits.
                    match k.code {
                        KeyCode::Esc | KeyCode::Char('s') => app.close_summary(),
                        KeyCode::Char('g') => app.regenerate_summary(),
                        KeyCode::Char('q') => break,
                        _ => {}
                    }
                } else {
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('j') | KeyCode::Down => app.select_next(),
                        KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
                        KeyCode::Char('r') => app.refresh(),
                        KeyCode::Char('s') => app.open_summary(),
                        KeyCode::Char('a') => app.approve_selected(true),
                        KeyCode::Char('d') => app.approve_selected(false),
                        KeyCode::Enter => app.open_approval(),
                        KeyCode::Char('i') => app.open_install(),
                        KeyCode::Char('K') => app.start_key_input(),
                        _ => {}
                    }
                }
            }
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}
