//! Application state: owns every known session reader, decides which are
//! "active", and keeps a stable selection across refreshes.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::anthropic;
use crate::bridge::{self, Pending};
use crate::session::{discover, Session, Status};

/// State of an AI summary for one session.
pub enum SummaryState {
    Loading,
    Done(String),
    Error(String),
}

fn key_file() -> PathBuf {
    bridge::base_dir().join("api_key")
}

/// Resolve the API key: `ANTHROPIC_API_KEY` wins, else the saved key file.
fn load_api_key() -> Option<String> {
    if let Some(k) = std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty()) {
        return Some(k);
    }
    std::fs::read_to_string(key_file())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Persist the key to `~/.claude/iris/api_key` with owner-only (0600) perms.
fn save_api_key(key: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(bridge::base_dir())?;
    let path = key_file();
    std::fs::write(&path, key)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub struct App {
    pub projects_dir: PathBuf,
    pub window: Duration,
    pub interval: Duration,

    readers: HashMap<PathBuf, Session>,
    /// Paths of currently active sessions, most-recently-active first.
    pub visible: Vec<PathBuf>,
    pub selected: usize,
    selected_path: Option<PathBuf>,

    last_refresh: Instant,
    pub should_quit: bool,

    // AI summaries
    api_key: Option<String>,
    pub summaries: HashMap<PathBuf, SummaryState>,
    pub popup_open: bool,
    summary_tx: Sender<(PathBuf, Result<String, String>)>,
    summary_rx: Receiver<(PathBuf, Result<String, String>)>,

    /// Pending tool-approval requests from the hook bridge, keyed by session id.
    pub pending: HashMap<String, Pending>,

    // API-key entry
    pub editing_key: bool,
    pub key_buffer: String,

    // Approval detail modal + AI risk assessment (keyed by request id)
    pub approve_open: bool,
    pub assessments: HashMap<String, SummaryState>,
    assess_tx: Sender<(String, Result<String, String>)>,
    assess_rx: Receiver<(String, Result<String, String>)>,

    /// Transient one-line status shown in the header.
    pub flash: Option<String>,

    /// Whether an `iris hook` is registered in settings.json.
    pub hook_installed: bool,
    /// The install/enable-approvals proposal modal is showing.
    pub install_open: bool,
    last_pending_logged: usize,
}

impl App {
    pub fn new(projects_dir: PathBuf, window: Duration, interval: Duration) -> Self {
        let (summary_tx, summary_rx) = channel();
        let (assess_tx, assess_rx) = channel();
        let mut app = App {
            projects_dir,
            window,
            interval,
            readers: HashMap::new(),
            visible: Vec::new(),
            selected: 0,
            selected_path: None,
            last_refresh: Instant::now(),
            should_quit: false,
            api_key: load_api_key(),
            summaries: HashMap::new(),
            popup_open: false,
            summary_tx,
            summary_rx,
            pending: HashMap::new(),
            editing_key: false,
            key_buffer: String::new(),
            approve_open: false,
            assessments: HashMap::new(),
            assess_tx,
            assess_rx,
            flash: None,
            hook_installed: bridge::hook_installed(),
            install_open: false,
            last_pending_logged: 0,
        };
        // Propose enabling approvals on first launch when the hook isn't set up.
        app.install_open = !app.hook_installed;
        bridge::touch_heartbeat();
        bridge::log(&format!(
            "iris start: api_key={} hook_installed={}",
            app.api_key.is_some(),
            app.hook_installed
        ));
        app.refresh();
        app
    }

    pub fn tick(&mut self) {
        self.drain_summaries();
        if self.last_refresh.elapsed() >= self.interval {
            self.refresh();
        }
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }

    pub fn open_install(&mut self) {
        self.install_open = true;
    }
    pub fn close_install(&mut self) {
        self.install_open = false;
    }

    /// Accept the proposal: register the hook so iris intercepts approvals.
    pub fn enable_approvals(&mut self) {
        match bridge::install_hook(false) {
            Ok(_) => {
                self.hook_installed = true;
                self.flash = Some("approvals ON — restart Claude sessions to arm it".into());
            }
            Err(e) => self.flash = Some(format!("enable failed: {e}")),
        }
        self.install_open = false;
    }

    /// Remove the hook so iris no longer intercepts other sessions.
    pub fn disable_approvals(&mut self) {
        match bridge::uninstall_hook(false) {
            Ok(_) => {
                self.hook_installed = false;
                self.flash = Some("approvals OFF — iris no longer intercepts sessions".into());
            }
            Err(e) => self.flash = Some(format!("disable failed: {e}")),
        }
        self.install_open = false;
    }

    pub fn start_key_input(&mut self) {
        self.editing_key = true;
        self.key_buffer.clear();
        self.flash = None;
    }

    pub fn cancel_key_input(&mut self) {
        self.editing_key = false;
        self.key_buffer.clear();
    }

    pub fn key_input_push(&mut self, c: char) {
        if !c.is_control() {
            self.key_buffer.push(c);
        }
    }

    pub fn key_input_backspace(&mut self) {
        self.key_buffer.pop();
    }

    /// Save the entered key (persisted 0600) and close the prompt.
    pub fn commit_key_input(&mut self) {
        let key = self.key_buffer.trim().to_string();
        self.editing_key = false;
        self.key_buffer.clear();
        if key.is_empty() {
            return;
        }
        match save_api_key(&key) {
            Ok(()) => {
                self.api_key = Some(key);
                self.flash = Some("API key saved to ~/.claude/iris/api_key".into());
            }
            Err(e) => self.flash = Some(format!("could not save key: {e}")),
        }
    }

    /// The pending request to act on: the selected session's, or — if it has
    /// none — the sole pending request anywhere (so `a`/`d` always do the
    /// obvious thing when only one approval is waiting).
    pub fn current_pending(&self) -> Option<&Pending> {
        if let Some(s) = self.selected_session() {
            if let Some(p) = self.pending.get(&s.id) {
                return Some(p);
            }
        }
        if self.pending.len() == 1 {
            return self.pending.values().next();
        }
        None
    }

    fn current_pending_id(&self) -> Option<String> {
        self.current_pending().map(|p| p.id.clone())
    }

    /// Write an allow/deny decision for the current pending request, with
    /// feedback in the header so the keypress is never silent.
    pub fn approve_selected(&mut self, allow: bool) {
        let target = self
            .current_pending()
            .map(|p| (p.session_id.clone(), p.id.clone(), p.tool_name.clone()));
        bridge::log(&format!(
            "approve_selected(allow={allow}): pending={} target={}",
            self.pending.len(),
            target.as_ref().map(|t| t.1.as_str()).unwrap_or("none")
        ));
        match target {
            Some((session_id, id, tool)) => {
                let verb = if allow { "approved" } else { "denied" };
                bridge::write_decision(&id, allow, &format!("{verb} via iris"));
                self.pending.remove(&session_id);
                self.assessments.remove(&id);
                self.approve_open = false;
                self.flash = Some(format!("{verb} {tool}"));
            }
            None => {
                self.flash = Some("nothing to approve (no pending tool request)".into());
            }
        }
    }

    /// Open the approval detail modal if there's a request to act on.
    pub fn open_approval(&mut self) {
        if self.current_pending().is_some() {
            self.approve_open = true;
        } else {
            self.flash = Some("nothing to approve (no pending tool request)".into());
        }
    }

    pub fn close_approval(&mut self) {
        self.approve_open = false;
    }

    /// Ask the model for a quick risk read on the pending tool call.
    pub fn assess_pending(&mut self) {
        let (id, prompt) = match self.current_pending() {
            Some(p) => (
                p.id.clone(),
                format!(
                    "A Claude Code agent in '{}' wants to run the tool '{}'.\nInput:\n{}\n\n\
In 2-3 short lines: what does this do, and how risky is it (destructive, \
irreversible, network, or secret-touching)? End with a final line exactly: \
RISK: low|medium|high",
                    p.cwd, p.tool_name, p.input
                ),
            ),
            None => return,
        };
        let key = match &self.api_key {
            Some(k) => k.clone(),
            None => {
                self.assessments.insert(
                    id,
                    SummaryState::Error("no API key — press K to set one".into()),
                );
                return;
            }
        };
        if matches!(self.assessments.get(&id), Some(SummaryState::Loading)) {
            return;
        }
        self.assessments.insert(id.clone(), SummaryState::Loading);
        let tx = self.assess_tx.clone();
        thread::spawn(move || {
            let result = anthropic::assess(&key, anthropic::SUMMARY_MODEL, &prompt);
            let _ = tx.send((id, result));
        });
    }

    /// Assessment state for the current pending request.
    pub fn current_assessment(&self) -> Option<&SummaryState> {
        self.current_pending_id()
            .and_then(|id| self.assessments.get(&id))
    }

    fn drain_summaries(&mut self) {
        while let Ok((path, result)) = self.summary_rx.try_recv() {
            let state = match result {
                Ok(text) => SummaryState::Done(text),
                Err(e) => SummaryState::Error(e),
            };
            self.summaries.insert(path, state);
        }
        while let Ok((id, result)) = self.assess_rx.try_recv() {
            let state = match result {
                Ok(text) => SummaryState::Done(text),
                Err(e) => SummaryState::Error(e),
            };
            self.assessments.insert(id, state);
        }
    }

    /// Open the summary popup for the selected session, kicking off generation
    /// if one isn't already cached or in flight.
    pub fn open_summary(&mut self) {
        self.popup_open = true;
        let path = match self.visible.get(self.selected) {
            Some(p) => p.clone(),
            None => return,
        };
        match self.summaries.get(&path) {
            Some(SummaryState::Loading) | Some(SummaryState::Done(_)) => {}
            _ => self.request_summary(path),
        }
    }

    pub fn close_summary(&mut self) {
        self.popup_open = false;
    }

    /// Force-regenerate the summary for the selected session.
    pub fn regenerate_summary(&mut self) {
        if let Some(path) = self.visible.get(self.selected).cloned() {
            if !matches!(self.summaries.get(&path), Some(SummaryState::Loading)) {
                self.request_summary(path);
            }
        }
    }

    fn request_summary(&mut self, path: PathBuf) {
        let key = match &self.api_key {
            Some(k) => k.clone(),
            None => {
                self.summaries.insert(
                    path,
                    SummaryState::Error(
                        "no API key — press K to set one (or export ANTHROPIC_API_KEY)".into(),
                    ),
                );
                return;
            }
        };
        let digest = match self.readers.get(&path) {
            Some(s) => s.digest(),
            None => return,
        };
        self.summaries.insert(path.clone(), SummaryState::Loading);
        let tx = self.summary_tx.clone();
        thread::spawn(move || {
            let result = anthropic::summarize(&key, anthropic::SUMMARY_MODEL, &digest);
            let _ = tx.send((path, result));
        });
    }

    pub fn selected_summary(&self) -> Option<&SummaryState> {
        self.visible
            .get(self.selected)
            .and_then(|p| self.summaries.get(p))
    }

    pub fn refresh(&mut self) {
        self.last_refresh = Instant::now();
        bridge::touch_heartbeat();
        self.hook_installed = bridge::hook_installed();

        // Load pending hook approvals, keyed by session id (newest per session).
        self.pending.clear();
        for p in bridge::load_pending() {
            match self.pending.get(&p.session_id) {
                Some(existing) if existing.ts >= p.ts => {}
                _ => {
                    self.pending.insert(p.session_id.clone(), p);
                }
            }
        }
        if self.pending.len() != self.last_pending_logged {
            bridge::log(&format!("pending requests: {}", self.pending.len()));
            self.last_pending_logged = self.pending.len();
        }

        for path in discover(&self.projects_dir) {
            let s = self
                .readers
                .entry(path.clone())
                .or_insert_with(|| Session::new(path.clone()));
            let _ = s.refresh();
        }

        let cutoff = SystemTime::now()
            .checked_sub(self.window)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Show anything touched within the window, plus any session blocked
        // waiting for approval (transcript heuristic OR a live hook request) —
        // those stay pinned however long they wait.
        let mut active: Vec<&Session> = self
            .readers
            .values()
            .filter(|s| {
                s.mtime >= cutoff
                    || s.status() == Status::NeedsApproval
                    || self.pending.contains_key(&s.id)
            })
            .collect();
        // Sort: live hook approvals first, then by status, then most recent.
        let rank = |s: &Session| -> (u8, u8) {
            let pend = if self.pending.contains_key(&s.id) { 0 } else { 1 };
            (pend, s.status().rank())
        };
        active.sort_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then(b.mtime.cmp(&a.mtime))
        });
        self.visible = active.iter().map(|s| s.path.clone()).collect();

        self.restore_selection();
    }

    fn restore_selection(&mut self) {
        if self.visible.is_empty() {
            self.selected = 0;
            self.selected_path = None;
            return;
        }
        if let Some(p) = &self.selected_path {
            if let Some(i) = self.visible.iter().position(|x| x == p) {
                self.selected = i;
                return;
            }
        }
        self.selected = self.selected.min(self.visible.len() - 1);
        self.selected_path = self.visible.get(self.selected).cloned();
    }

    pub fn select_next(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.visible.len() - 1);
        self.selected_path = self.visible.get(self.selected).cloned();
    }

    pub fn select_prev(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
        self.selected_path = self.visible.get(self.selected).cloned();
    }

    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.visible.iter().filter_map(move |p| self.readers.get(p))
    }

    pub fn selected_session(&self) -> Option<&Session> {
        self.visible
            .get(self.selected)
            .and_then(|p| self.readers.get(p))
    }
}
