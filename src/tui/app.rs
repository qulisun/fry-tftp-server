use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction as LayoutDirection, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Sparkline, Table, Tabs,
    Wrap,
};
use ratatui::Frame;

use crate::core::log_buffer::{LogBuffer, LogEntry};
use crate::core::state::*;

// ─── Tab ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Dashboard,
    Files,
    Transfers,
    Log,
    Config,
    Acl,
    Help,
}

impl Tab {
    const ALL: &'static [Tab] = &[
        Tab::Dashboard,
        Tab::Files,
        Tab::Transfers,
        Tab::Log,
        Tab::Config,
        Tab::Acl,
        Tab::Help,
    ];

    fn title(&self) -> &str {
        match self {
            Tab::Dashboard => " 1:Dashboard ",
            Tab::Files => " 2:Files ",
            Tab::Transfers => " 3:Transfers ",
            Tab::Log => " 4:Log ",
            Tab::Config => " 5:Config ",
            Tab::Acl => " 6:ACL ",
            Tab::Help => " 7:Help ",
        }
    }

    fn index(&self) -> usize {
        match self {
            Tab::Dashboard => 0,
            Tab::Files => 1,
            Tab::Transfers => 2,
            Tab::Log => 3,
            Tab::Config => 4,
            Tab::Acl => 5,
            Tab::Help => 6,
        }
    }
}

// ─── ACL Edit State ──────────────────────────────────────────────────────────

#[derive(Clone)]
struct AclRuleEdit {
    action: String,
    source: String,
    operations: String,
    comment: String,
}

enum AclEditMode {
    None,
    Adding(AclRuleEdit, usize), // rule being built, cursor field (0-3)
    Editing(usize, AclRuleEdit, usize), // rule index, edit buffer, cursor field
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1}GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1}MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1}KB", bytes as f64 / 1_000.0)
    } else {
        format!("{}B", bytes)
    }
}

fn format_rate(bps: f64) -> String {
    if bps >= 1_000_000.0 {
        format!("{:.1} MB/s", bps / 1_000_000.0)
    } else if bps >= 1_000.0 {
        format!("{:.1} KB/s", bps / 1_000.0)
    } else {
        format!("{:.0} B/s", bps)
    }
}

fn format_duration_short(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}h{}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m{}s", s / 60, s % 60)
    } else {
        format!("{}s", s)
    }
}

fn log_level_color(level: &tracing::Level) -> Color {
    match *level {
        tracing::Level::ERROR => Color::Red,
        tracing::Level::WARN => Color::Yellow,
        tracing::Level::INFO => Color::Green,
        tracing::Level::DEBUG => Color::Cyan,
        tracing::Level::TRACE => Color::DarkGray,
    }
}

fn format_log_time(ts: std::time::SystemTime) -> String {
    let dur = ts.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let s = dur.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        (s % 86400) / 3600,
        (s % 3600) / 60,
        s % 60
    )
}

// ─── TuiApp ──────────────────────────────────────────────────────────────────

pub struct TuiApp {
    state: Arc<AppState>,
    log_buffer: LogBuffer,
    current_tab: Tab,
    pub should_quit: bool,
    start_time: Instant,

    // Bandwidth
    bw_tx: VecDeque<u64>,
    bw_rx: VecDeque<u64>,
    prev_tx: u64,
    prev_rx: u64,
    cur_tx_rate: f64,
    cur_rx_rate: f64,
    last_sample: Instant,

    // Log
    log_entries: Vec<LogEntry>,
    log_scroll: usize,
    log_auto_scroll: bool,

    // Scroll positions
    active_scroll: usize,
    transfers_scroll: usize,
    files_scroll: usize,
    config_scroll: usize,
    acl_scroll: usize,

    // Files
    files_entries: Vec<(String, bool, u64)>,
    files_root: std::path::PathBuf,
    files_dirty: bool,

    // Config
    config_items: Vec<(String, String, String)>,
    config_editing: Option<usize>,
    config_edit_buf: String,

    // ACL editing (D1)
    acl_edit_mode: AclEditMode,

    // Search/filter (D2)
    filter_active: bool,
    filter_text: String,

    show_help: bool,
}

impl TuiApp {
    pub fn new(state: Arc<AppState>, log_buffer: LogBuffer) -> Self {
        let config = state.config();
        let root = config.server.root.clone();

        let mut app = Self {
            state,
            log_buffer,
            current_tab: Tab::Dashboard,
            should_quit: false,
            start_time: Instant::now(),
            bw_tx: VecDeque::with_capacity(60),
            bw_rx: VecDeque::with_capacity(60),
            prev_tx: 0,
            prev_rx: 0,
            cur_tx_rate: 0.0,
            cur_rx_rate: 0.0,
            last_sample: Instant::now(),
            log_entries: Vec::new(),
            log_scroll: 0,
            log_auto_scroll: true,
            active_scroll: 0,
            transfers_scroll: 0,
            files_scroll: 0,
            config_scroll: 0,
            acl_scroll: 0,
            files_entries: Vec::new(),
            files_root: root,
            files_dirty: true,
            config_items: Vec::new(),
            config_editing: None,
            config_edit_buf: String::new(),
            acl_edit_mode: AclEditMode::None,
            filter_active: false,
            filter_text: String::new(),
            show_help: false,
        };
        app.refresh_config();
        app
    }

    // ─── Data updates ────────────────────────────────────────────────────────

    pub fn tick(&mut self) {
        // Bandwidth
        let now = Instant::now();
        if now.duration_since(self.last_sample).as_millis() >= 1000 {
            let tx = self.state.total_bytes_tx.load(Ordering::Relaxed);
            let rx = self.state.total_bytes_rx.load(Ordering::Relaxed);
            let dt = now.duration_since(self.last_sample).as_secs_f64();
            self.cur_tx_rate = tx.saturating_sub(self.prev_tx) as f64 / dt;
            self.cur_rx_rate = rx.saturating_sub(self.prev_rx) as f64 / dt;
            self.bw_tx
                .push_back((self.cur_tx_rate / 1024.0).max(0.0) as u64);
            self.bw_rx
                .push_back((self.cur_rx_rate / 1024.0).max(0.0) as u64);
            if self.bw_tx.len() > 60 {
                self.bw_tx.pop_front();
            }
            if self.bw_rx.len() > 60 {
                self.bw_rx.pop_front();
            }
            self.prev_tx = tx;
            self.prev_rx = rx;
            self.last_sample = now;
        }

        // Logs
        if let Ok(mut buf) = self.log_buffer.lock() {
            while let Some(entry) = buf.pop_front() {
                self.log_entries.push(entry);
            }
            if self.log_entries.len() > 10000 {
                let excess = self.log_entries.len() - 10000;
                self.log_entries.drain(0..excess);
            }
        }

        // Files
        if self.files_dirty {
            self.refresh_files();
        }
    }

    fn refresh_files(&mut self) {
        self.files_entries.clear();
        if let Ok(rd) = std::fs::read_dir(&self.files_root) {
            let mut entries: Vec<_> = rd
                .filter_map(|e| e.ok())
                .map(|e| {
                    let m = e.metadata().ok();
                    (
                        e.file_name().to_string_lossy().to_string(),
                        m.as_ref().is_some_and(|m| m.is_dir()),
                        m.as_ref().map_or(0, |m| m.len()),
                    )
                })
                .collect();
            entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            self.files_entries = entries;
        }
        self.files_dirty = false;
    }

    fn refresh_config(&mut self) {
        let c = self.state.config();
        self.config_items = vec![
            ("Server".into(), "port".into(), c.server.port.to_string()),
            (
                "Server".into(),
                "bind_address".into(),
                c.server.bind_address.clone(),
            ),
            (
                "Server".into(),
                "root".into(),
                c.server.root.to_string_lossy().to_string(),
            ),
            (
                "Server".into(),
                "log_level".into(),
                c.server.log_level.clone(),
            ),
            (
                "Protocol".into(),
                "allow_write".into(),
                c.protocol.allow_write.to_string(),
            ),
            (
                "Protocol".into(),
                "blksize".into(),
                c.protocol.default_blksize.to_string(),
            ),
            (
                "Protocol".into(),
                "max_blksize".into(),
                c.protocol.max_blksize.to_string(),
            ),
            (
                "Protocol".into(),
                "windowsize".into(),
                c.protocol.default_windowsize.to_string(),
            ),
            (
                "Protocol".into(),
                "max_windowsize".into(),
                c.protocol.max_windowsize.to_string(),
            ),
            (
                "Protocol".into(),
                "timeout".into(),
                c.protocol.default_timeout.to_string(),
            ),
            (
                "Session".into(),
                "max_sessions".into(),
                c.session.max_sessions.to_string(),
            ),
            (
                "Session".into(),
                "max_retries".into(),
                c.session.max_retries.to_string(),
            ),
            (
                "Security".into(),
                "per_ip_max".into(),
                c.security.per_ip_max_sessions.to_string(),
            ),
            (
                "Security".into(),
                "rate_limit".into(),
                c.security.per_ip_rate_limit.to_string(),
            ),
            (
                "Filesystem".into(),
                "max_file_size".into(),
                c.filesystem.max_file_size.clone(),
            ),
            (
                "Filesystem".into(),
                "overwrite".into(),
                c.filesystem.allow_overwrite.to_string(),
            ),
        ];
    }

    fn apply_config_edit(&self, idx: usize) {
        if idx >= self.config_items.len() {
            return;
        }
        let (_, key, val) = &self.config_items[idx];
        let mut cfg = (*self.state.config()).clone();
        match key.as_str() {
            "port" => {
                if let Ok(v) = val.parse() {
                    cfg.server.port = v;
                }
            }
            "bind_address" => cfg.server.bind_address = val.clone(),
            "log_level" => cfg.server.log_level = val.clone(),
            "allow_write" => {
                if let Ok(v) = val.parse() {
                    cfg.protocol.allow_write = v;
                }
            }
            "blksize" => {
                if let Ok(v) = val.parse() {
                    cfg.protocol.default_blksize = v;
                }
            }
            "max_blksize" => {
                if let Ok(v) = val.parse() {
                    cfg.protocol.max_blksize = v;
                }
            }
            "windowsize" => {
                if let Ok(v) = val.parse() {
                    cfg.protocol.default_windowsize = v;
                }
            }
            "max_windowsize" => {
                if let Ok(v) = val.parse() {
                    cfg.protocol.max_windowsize = v;
                }
            }
            "timeout" => {
                if let Ok(v) = val.parse() {
                    cfg.protocol.default_timeout = v;
                }
            }
            "max_sessions" => {
                if let Ok(v) = val.parse() {
                    cfg.session.max_sessions = v;
                }
            }
            "max_retries" => {
                if let Ok(v) = val.parse() {
                    cfg.session.max_retries = v;
                }
            }
            "per_ip_max" => {
                if let Ok(v) = val.parse() {
                    cfg.security.per_ip_max_sessions = v;
                }
            }
            "rate_limit" => {
                if let Ok(v) = val.parse() {
                    cfg.security.per_ip_rate_limit = v;
                }
            }
            "max_file_size" => cfg.filesystem.max_file_size = val.clone(),
            "overwrite" => {
                if let Ok(v) = val.parse() {
                    cfg.filesystem.allow_overwrite = v;
                }
            }
            _ => {}
        }
        let _ = cfg.save();
        self.state.config.store(Arc::new(cfg));
    }

    fn save_acl_rule(&self, idx: Option<usize>, rule: &AclRuleEdit) {
        use crate::core::config::AclRuleConfig;
        let mut cfg = (*self.state.config()).clone();
        let new_rule = AclRuleConfig {
            action: rule.action.clone(),
            source: rule.source.clone(),
            operations: rule
                .operations
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            comment: rule.comment.clone(),
        };
        match idx {
            Some(i) if i < cfg.acl.rules.len() => cfg.acl.rules[i] = new_rule,
            _ => cfg.acl.rules.push(new_rule),
        }
        let _ = cfg.save();
        self.state.config.store(Arc::new(cfg));
    }

    fn delete_acl_rule(&self, idx: usize) {
        let mut cfg = (*self.state.config()).clone();
        if idx < cfg.acl.rules.len() {
            cfg.acl.rules.remove(idx);
            let _ = cfg.save();
            self.state.config.store(Arc::new(cfg));
        }
    }

    // ─── Input ───────────────────────────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Filter input mode
        if self.filter_active {
            match key.code {
                KeyCode::Esc => {
                    self.filter_active = false;
                    self.filter_text.clear();
                }
                KeyCode::Enter => {
                    self.filter_active = false;
                    // Keep filter_text active for display
                }
                KeyCode::Char(c) => self.filter_text.push(c),
                KeyCode::Backspace => {
                    self.filter_text.pop();
                }
                _ => {}
            }
            return false;
        }

        // ACL edit mode
        if !matches!(self.acl_edit_mode, AclEditMode::None) {
            match key.code {
                KeyCode::Esc => {
                    self.acl_edit_mode = AclEditMode::None;
                    return false;
                }
                KeyCode::Tab => {
                    match &mut self.acl_edit_mode {
                        AclEditMode::Adding(_, field) | AclEditMode::Editing(_, _, field) => {
                            *field = (*field + 1) % 4
                        }
                        _ => {}
                    }
                    return false;
                }
                KeyCode::BackTab => {
                    match &mut self.acl_edit_mode {
                        AclEditMode::Adding(_, field) | AclEditMode::Editing(_, _, field) => {
                            *field = (*field + 3) % 4
                        }
                        _ => {}
                    }
                    return false;
                }
                KeyCode::Enter => {
                    let (idx, rule_clone) = match &self.acl_edit_mode {
                        AclEditMode::Adding(rule, _) => (None, rule.clone()),
                        AclEditMode::Editing(i, rule, _) => (Some(*i), rule.clone()),
                        AclEditMode::None => unreachable!(),
                    };
                    self.save_acl_rule(idx, &rule_clone);
                    self.acl_edit_mode = AclEditMode::None;
                    return false;
                }
                KeyCode::Char(c) => {
                    match &mut self.acl_edit_mode {
                        AclEditMode::Adding(rule, field) | AclEditMode::Editing(_, rule, field) => {
                            let buf = match *field {
                                0 => &mut rule.action,
                                1 => &mut rule.source,
                                2 => &mut rule.operations,
                                _ => &mut rule.comment,
                            };
                            buf.push(c);
                        }
                        _ => {}
                    }
                    return false;
                }
                KeyCode::Backspace => {
                    match &mut self.acl_edit_mode {
                        AclEditMode::Adding(rule, field) | AclEditMode::Editing(_, rule, field) => {
                            let buf = match *field {
                                0 => &mut rule.action,
                                1 => &mut rule.source,
                                2 => &mut rule.operations,
                                _ => &mut rule.comment,
                            };
                            buf.pop();
                        }
                        _ => {}
                    }
                    return false;
                }
                _ => return false,
            }
        }

        // Config editing mode
        if let Some(idx) = self.config_editing {
            match key.code {
                KeyCode::Esc => self.config_editing = None,
                KeyCode::Enter => {
                    if idx < self.config_items.len() {
                        self.config_items[idx].2 = self.config_edit_buf.clone();
                        self.apply_config_edit(idx);
                    }
                    self.config_editing = None;
                }
                KeyCode::Char(c) => self.config_edit_buf.push(c),
                KeyCode::Backspace => {
                    self.config_edit_buf.pop();
                }
                _ => {}
            }
            return false;
        }

        // Help overlay
        if self.show_help {
            self.show_help = false;
            return false;
        }

        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('s') => self.state.cancel_shutdown(),
            KeyCode::Char('r') => {
                self.refresh_config();
                self.files_dirty = true;
            }
            KeyCode::Char('/') => {
                self.filter_active = true;
                self.filter_text.clear();
            }
            KeyCode::Char('1') => self.current_tab = Tab::Dashboard,
            KeyCode::Char('2') => {
                self.current_tab = Tab::Files;
                self.files_dirty = true;
            }
            KeyCode::Char('3') => self.current_tab = Tab::Transfers,
            KeyCode::Char('4') => self.current_tab = Tab::Log,
            KeyCode::Char('5') => self.current_tab = Tab::Config,
            KeyCode::Char('6') => self.current_tab = Tab::Acl,
            KeyCode::Char('7') => self.current_tab = Tab::Help,
            KeyCode::Tab => {
                let i = self.current_tab.index();
                self.current_tab = Tab::ALL[(i + 1) % Tab::ALL.len()];
            }
            KeyCode::BackTab => {
                let i = self.current_tab.index();
                self.current_tab = Tab::ALL[(i + Tab::ALL.len() - 1) % Tab::ALL.len()];
            }
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(),
            KeyCode::Enter => self.handle_enter(),
            // ACL: 'a' to add, 'e' to edit, 'd' to delete
            KeyCode::Char('a') if self.current_tab == Tab::Acl => {
                self.acl_edit_mode = AclEditMode::Adding(
                    AclRuleEdit {
                        action: "allow".into(),
                        source: String::new(),
                        operations: "read".into(),
                        comment: String::new(),
                    },
                    0,
                );
            }
            KeyCode::Char('e') if self.current_tab == Tab::Acl => {
                let config = self.state.config();
                if let Some(r) = config.acl.rules.get(self.acl_scroll) {
                    self.acl_edit_mode = AclEditMode::Editing(
                        self.acl_scroll,
                        AclRuleEdit {
                            action: r.action.clone(),
                            source: r.source.clone(),
                            operations: r.operations.join(", "),
                            comment: r.comment.clone(),
                        },
                        0,
                    );
                }
            }
            KeyCode::Char('d') if self.current_tab == Tab::Acl => {
                self.delete_acl_rule(self.acl_scroll);
                if self.acl_scroll > 0 {
                    self.acl_scroll -= 1;
                }
            }
            KeyCode::Esc => {
                if !self.filter_text.is_empty() {
                    self.filter_text.clear();
                } else if self.current_tab == Tab::Files {
                    if let Some(parent) = self.files_root.parent() {
                        self.files_root = parent.to_path_buf();
                        self.files_dirty = true;
                        self.files_scroll = 0;
                    }
                }
            }
            _ => {}
        }
        false
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_up(),
            MouseEventKind::ScrollDown => self.scroll_down(),
            _ => {}
        }
    }

    fn scroll_up(&mut self) {
        match self.current_tab {
            Tab::Dashboard => self.active_scroll = self.active_scroll.saturating_sub(1),
            Tab::Files => self.files_scroll = self.files_scroll.saturating_sub(1),
            Tab::Transfers => self.transfers_scroll = self.transfers_scroll.saturating_sub(1),
            Tab::Log => {
                self.log_auto_scroll = false;
                self.log_scroll = self.log_scroll.saturating_sub(1);
            }
            Tab::Config => self.config_scroll = self.config_scroll.saturating_sub(1),
            Tab::Acl => self.acl_scroll = self.acl_scroll.saturating_sub(1),
            Tab::Help => {} // static content, no scroll state
        }
    }

    fn scroll_down(&mut self) {
        match self.current_tab {
            Tab::Dashboard => self.active_scroll += 1,
            Tab::Files => {
                if self.files_scroll < self.files_entries.len().saturating_sub(1) {
                    self.files_scroll += 1;
                }
            }
            Tab::Transfers => self.transfers_scroll += 1,
            Tab::Log => {
                if self.log_scroll < self.log_entries.len().saturating_sub(1) {
                    self.log_scroll += 1;
                }
                if self.log_scroll >= self.log_entries.len().saturating_sub(3) {
                    self.log_auto_scroll = true;
                }
            }
            Tab::Config => {
                if self.config_scroll < self.config_items.len().saturating_sub(1) {
                    self.config_scroll += 1;
                }
            }
            Tab::Acl => self.acl_scroll += 1,
            Tab::Help => {} // static content, no scroll state
        }
    }

    fn handle_enter(&mut self) {
        match self.current_tab {
            Tab::Files => {
                if let Some(entry) = self.files_entries.get(self.files_scroll) {
                    if entry.1 {
                        let name = entry.0.clone();
                        self.files_root = self.files_root.join(&name);
                        self.files_dirty = true;
                        self.files_scroll = 0;
                    }
                }
            }
            Tab::Config => {
                if self.config_scroll < self.config_items.len() {
                    self.config_edit_buf = self.config_items[self.config_scroll].2.clone();
                    self.config_editing = Some(self.config_scroll);
                }
            }
            _ => {}
        }
    }

    fn matches_filter(&self, text: &str) -> bool {
        self.filter_text.is_empty()
            || text
                .to_lowercase()
                .contains(&self.filter_text.to_lowercase())
    }

    // ─── Render ──────────────────────────────────────────────────────────────

    pub fn render(&mut self, frame: &mut Frame) {
        self.tick();

        let has_filter = !self.filter_text.is_empty() || self.filter_active;
        let bottom_height = if has_filter { 2 } else { 1 };

        let chunks = Layout::default()
            .direction(LayoutDirection::Vertical)
            .constraints([
                Constraint::Length(3),             // tabs
                Constraint::Min(0),                // content
                Constraint::Length(bottom_height), // status + filter
            ])
            .split(frame.area());

        // Tab bar
        let titles: Vec<Line> = Tab::ALL.iter().map(|t| Line::from(t.title())).collect();
        let server_state = self.state.get_server_state();
        let status_str = match server_state {
            ServerState::Running => "Running",
            ServerState::Stopped => "Stopped",
            ServerState::Starting => "Starting",
            ServerState::Stopping => "Stopping",
            ServerState::Error => "Error",
        };
        let tabs = Tabs::new(titles)
            .select(self.current_tab.index())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Fry TFTP Server [{}] ", status_str)),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_widget(tabs, chunks[0]);

        // Content
        match self.current_tab {
            Tab::Dashboard => self.render_dashboard(frame, chunks[1]),
            Tab::Files => self.render_files(frame, chunks[1]),
            Tab::Transfers => self.render_transfers(frame, chunks[1]),
            Tab::Log => self.render_log(frame, chunks[1]),
            Tab::Config => self.render_config(frame, chunks[1]),
            Tab::Acl => self.render_acl(frame, chunks[1]),
            Tab::Help => self.render_help(frame, chunks[1]),
        }

        // Status bar
        if has_filter {
            let bar_chunks = Layout::default()
                .direction(LayoutDirection::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1)])
                .split(chunks[2]);

            let filter_line = if self.filter_active {
                format!("/{}_", self.filter_text)
            } else {
                format!("Filter: {} (Esc to clear)", self.filter_text)
            };
            let filter_bar = Paragraph::new(filter_line)
                .style(Style::default().bg(Color::Blue).fg(Color::White));
            frame.render_widget(filter_bar, bar_chunks[0]);

            let hint = self.status_hint();
            let bar =
                Paragraph::new(hint).style(Style::default().bg(Color::DarkGray).fg(Color::White));
            frame.render_widget(bar, bar_chunks[1]);
        } else {
            let hint = self.status_hint();
            let bar =
                Paragraph::new(hint).style(Style::default().bg(Color::DarkGray).fg(Color::White));
            frame.render_widget(bar, chunks[2]);
        }

        // Overlays
        if self.show_help {
            self.render_help_overlay(frame);
        }
        if let Some(idx) = self.config_editing {
            self.render_edit_popup(frame, idx);
        }
        match &self.acl_edit_mode {
            AclEditMode::Adding(rule, field) => {
                self.render_acl_edit_popup(frame, "Add ACL Rule", rule, *field);
            }
            AclEditMode::Editing(_, rule, field) => {
                self.render_acl_edit_popup(frame, "Edit ACL Rule", rule, *field);
            }
            AclEditMode::None => {}
        }
    }

    fn status_hint(&self) -> &str {
        if self.config_editing.is_some() {
            " Enter: save | Esc: cancel "
        } else if matches!(
            self.acl_edit_mode,
            AclEditMode::Adding(..) | AclEditMode::Editing(..)
        ) {
            " Tab: next field | Enter: save | Esc: cancel "
        } else if self.current_tab == Tab::Acl {
            " a: add | e: edit | d: delete | /: filter | q: quit | ?: help "
        } else {
            " 1-6: tabs | j/k: scroll | /: filter | Enter: select | s: stop | r: reload | q: quit | ?: help "
        }
    }

    // ─── Dashboard ───────────────────────────────────────────────────────────

    fn render_dashboard(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(LayoutDirection::Vertical)
            .constraints([
                Constraint::Length(5), // stats
                Constraint::Length(4), // sparklines
                Constraint::Min(0),    // active transfers
            ])
            .split(area);

        // Stats
        let config = self.state.config();
        let uptime = format_duration_short(self.start_time.elapsed());
        let total = self.state.total_sessions.load(Ordering::Relaxed);
        let errors = self.state.total_errors.load(Ordering::Relaxed);
        let tx = self.state.total_bytes_tx.load(Ordering::Relaxed);
        let rx = self.state.total_bytes_rx.load(Ordering::Relaxed);

        let stats_text = vec![
            Line::from(vec![
                Span::styled("Bind: ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!(
                    "{}:{}",
                    config.server.bind_address, config.server.port
                )),
                Span::raw("  "),
                Span::styled("Uptime: ", Style::default().fg(Color::DarkGray)),
                Span::raw(&uptime),
                Span::raw("  "),
                Span::styled("Root: ", Style::default().fg(Color::DarkGray)),
                Span::raw(config.server.root.to_string_lossy().to_string()),
            ]),
            Line::from(vec![
                Span::styled("Sessions: ", Style::default().fg(Color::DarkGray)),
                Span::raw(total.to_string()),
                Span::raw("  "),
                Span::styled("Errors: ", Style::default().fg(Color::DarkGray)),
                Span::styled(errors.to_string(), Style::default().fg(Color::Red)),
                Span::raw("  "),
                Span::styled("TX: ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!(
                    "{} ({})",
                    format_bytes(tx),
                    format_rate(self.cur_tx_rate)
                )),
                Span::raw("  "),
                Span::styled("RX: ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!(
                    "{} ({})",
                    format_bytes(rx),
                    format_rate(self.cur_rx_rate)
                )),
            ]),
        ];
        let stats = Paragraph::new(stats_text)
            .block(Block::default().borders(Borders::ALL).title(" Status "));
        frame.render_widget(stats, chunks[0]);

        // Sparklines
        let spark_chunks = Layout::default()
            .direction(LayoutDirection::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);

        let tx_data: Vec<u64> = self.bw_tx.iter().copied().collect();
        let tx_spark = Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" TX {} ", format_rate(self.cur_tx_rate))),
            )
            .data(&tx_data)
            .style(Style::default().fg(Color::Cyan));
        frame.render_widget(tx_spark, spark_chunks[0]);

        let rx_data: Vec<u64> = self.bw_rx.iter().copied().collect();
        let rx_spark = Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" RX {} ", format_rate(self.cur_rx_rate))),
            )
            .data(&rx_data)
            .style(Style::default().fg(Color::Green));
        frame.render_widget(rx_spark, spark_chunks[1]);

        // Active transfers table
        let sessions: Vec<SessionInfo> = self
            .state
            .active_sessions
            .try_read()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default();

        let header = Row::new(vec!["Client", "File", "Dir", "Progress", "Speed", "Time"])
            .style(Style::default().add_modifier(Modifier::BOLD))
            .bottom_margin(0);

        let rows: Vec<Row> = sessions
            .iter()
            .filter(|s| {
                self.matches_filter(&s.filename) || self.matches_filter(&s.client_addr.to_string())
            })
            .map(|s| {
                let dir = match s.direction {
                    Direction::Read => "DL",
                    Direction::Write => "UL",
                };
                let progress = if let Some(tsize) = s.tsize {
                    if tsize > 0 {
                        format!(
                            "{:.0}% {}",
                            s.bytes_transferred as f64 / tsize as f64 * 100.0,
                            format_bytes(s.bytes_transferred)
                        )
                    } else {
                        format_bytes(s.bytes_transferred)
                    }
                } else {
                    format_bytes(s.bytes_transferred)
                };
                let elapsed = s.started_at.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 {
                    s.bytes_transferred as f64 / elapsed
                } else {
                    0.0
                };
                Row::new(vec![
                    s.client_addr.to_string(),
                    s.filename.clone(),
                    dir.to_string(),
                    progress,
                    format_rate(speed),
                    format_duration_short(s.started_at.elapsed()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(22),
            Constraint::Min(15),
            Constraint::Length(4),
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(8),
        ];
        let table = Table::new(rows, widths).header(header).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Active Transfers ({}) ", sessions.len())),
        );
        frame.render_widget(table, chunks[2]);
    }

    // ─── Files ───────────────────────────────────────────────────────────────

    fn render_files(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .files_entries
            .iter()
            .enumerate()
            .filter(|(_, (name, _, _))| self.matches_filter(name))
            .map(|(i, (name, is_dir, size))| {
                let content = if *is_dir {
                    format!("[DIR] {}/", name)
                } else {
                    format!("      {}  {}", name, format_bytes(*size))
                };
                let style = if i == self.files_scroll {
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else if *is_dir {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                };
                ListItem::new(content).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(format!(
                " Files: {} (Esc: up, Enter: open) ",
                self.files_root.display()
            )))
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            );

        let mut list_state = ListState::default();
        list_state.select(Some(self.files_scroll));
        frame.render_stateful_widget(list, area, &mut list_state);
    }

    // ─── Transfers ───────────────────────────────────────────────────────────

    fn render_transfers(&self, frame: &mut Frame, area: Rect) {
        let history: Vec<TransferRecord> = self
            .state
            .transfer_history
            .try_read()
            .map(|h| h.clone())
            .unwrap_or_default();

        let header = Row::new(vec![
            "Client", "File", "Dir", "Size", "Duration", "Speed", "Status", "Retx",
        ])
        .style(Style::default().add_modifier(Modifier::BOLD));

        let rows: Vec<Row> = history
            .iter()
            .rev()
            .filter(|r| {
                self.matches_filter(&r.filename) || self.matches_filter(&r.client_addr.to_string())
            })
            .map(|r| {
                let dir = match r.direction {
                    Direction::Read => "DL",
                    Direction::Write => "UL",
                };
                let status_style = match r.status {
                    SessionStatus::Completed => Style::default().fg(Color::Green),
                    SessionStatus::Failed => Style::default().fg(Color::Red),
                    _ => Style::default().fg(Color::Yellow),
                };
                let status = match r.status {
                    SessionStatus::Completed => "OK",
                    SessionStatus::Failed => "FAIL",
                    SessionStatus::Cancelled => "CANCEL",
                    _ => "?",
                };
                Row::new(vec![
                    Cell::from(r.client_addr.to_string()),
                    Cell::from(r.filename.clone()),
                    Cell::from(dir),
                    Cell::from(format_bytes(r.bytes_transferred)),
                    Cell::from(format!("{}ms", r.duration_ms)),
                    Cell::from(format!("{:.1}Mbps", r.speed_mbps)),
                    Cell::from(Span::styled(status, status_style)),
                    Cell::from(r.retransmits.to_string()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(22),
            Constraint::Min(12),
            Constraint::Length(4),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(5),
        ];
        let table = Table::new(rows, widths).header(header).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Transfer History ({}) ", history.len())),
        );
        frame.render_widget(table, area);
    }

    // ─── Log ─────────────────────────────────────────────────────────────────

    fn render_log(&mut self, frame: &mut Frame, area: Rect) {
        let height = area.height.saturating_sub(2) as usize;

        let filtered: Vec<&LogEntry> = self
            .log_entries
            .iter()
            .filter(|e| {
                self.filter_text.is_empty()
                    || e.message
                        .to_lowercase()
                        .contains(&self.filter_text.to_lowercase())
                    || e.target
                        .to_lowercase()
                        .contains(&self.filter_text.to_lowercase())
            })
            .collect();

        if self.log_auto_scroll && !filtered.is_empty() {
            self.log_scroll = filtered.len().saturating_sub(height);
        }

        let visible: Vec<Line> = filtered
            .iter()
            .skip(self.log_scroll)
            .take(height)
            .map(|e| {
                let color = log_level_color(&e.level);
                Line::from(vec![
                    Span::styled(
                        format!("{} ", format_log_time(e.timestamp)),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(format!("{:5} ", e.level), Style::default().fg(color)),
                    Span::styled(
                        format!("[{}] ", e.target),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(&e.message),
                ])
            })
            .collect();

        let auto = if self.log_auto_scroll {
            "auto"
        } else {
            "manual"
        };
        let log = Paragraph::new(visible).block(Block::default().borders(Borders::ALL).title(
            format!(" Log ({} entries, scroll: {}) ", filtered.len(), auto),
        ));
        frame.render_widget(log, area);
    }

    // ─── Config ──────────────────────────────────────────────────────────────

    fn render_config(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .config_items
            .iter()
            .enumerate()
            .filter(|(_, (group, key, val))| {
                self.matches_filter(group) || self.matches_filter(key) || self.matches_filter(val)
            })
            .map(|(i, (group, key, val))| {
                let style = if i == self.config_scroll {
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(format!("[{}] {} = {}", group, key, val)).style(style)
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Config (Enter: edit, r: reload) "),
        );
        frame.render_widget(list, area);
    }

    // ─── ACL ─────────────────────────────────────────────────────────────────

    fn render_acl(&self, frame: &mut Frame, area: Rect) {
        let config = self.state.config();
        let acl = &config.acl;

        let header = Row::new(vec!["#", "Action", "Source", "Operations", "Comment"])
            .style(Style::default().add_modifier(Modifier::BOLD));

        let rows: Vec<Row> = acl
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                self.matches_filter(&r.source)
                    || self.matches_filter(&r.action)
                    || self.matches_filter(&r.comment)
            })
            .map(|(i, r)| {
                let style = if i == self.acl_scroll {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from((i + 1).to_string()),
                    Cell::from(r.action.clone()),
                    Cell::from(r.source.clone()),
                    Cell::from(r.operations.join(",")),
                    Cell::from(r.comment.clone()),
                ])
                .style(style)
            })
            .collect();

        let widths = [
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Length(20),
            Constraint::Length(12),
            Constraint::Min(10),
        ];
        let table = Table::new(rows, widths).header(header).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" ACL (mode: {}) [a:add e:edit d:del] ", acl.mode)),
        );
        frame.render_widget(table, area);
    }

    // ─── ACL Edit Popup ──────────────────────────────────────────────────────

    fn render_acl_edit_popup(
        &self,
        frame: &mut Frame,
        title: &str,
        rule: &AclRuleEdit,
        active_field: usize,
    ) {
        let area = centered_rect(60, 40, frame.area());
        frame.render_widget(Clear, area);

        let fields = ["Action", "Source (CIDR)", "Operations", "Comment"];
        let values = [&rule.action, &rule.source, &rule.operations, &rule.comment];

        let mut lines = vec![Line::from(format!(" {}", title)), Line::from("")];

        for (i, (field, val)) in fields.iter().zip(values.iter()).enumerate() {
            let marker = if i == active_field { ">" } else { " " };
            let cursor = if i == active_field { "_" } else { "" };
            lines.push(Line::from(format!(
                "{} {}: {}{}",
                marker, field, val, cursor
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(" Tab: next field | Enter: save | Esc: cancel"));

        let popup = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", title)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(popup, area);
    }

    // ─── Help Overlay ────────────────────────────────────────────────────────

    fn render_help_overlay(&self, frame: &mut Frame) {
        let area = centered_rect(50, 60, frame.area());
        frame.render_widget(Clear, area);

        let help = vec![
            Line::from("Key Bindings:"),
            Line::from(""),
            Line::from("  1-6         Switch tab"),
            Line::from("  Tab/S-Tab   Next/prev tab"),
            Line::from("  j/k, Up/Dn  Scroll"),
            Line::from("  Enter       Select/edit"),
            Line::from("  Esc         Back/cancel/clear filter"),
            Line::from("  /           Search/filter"),
            Line::from("  s           Stop server"),
            Line::from("  r           Reload config"),
            Line::from("  q           Quit"),
            Line::from("  ?           This help"),
            Line::from(""),
            Line::from("ACL tab:"),
            Line::from("  a           Add rule"),
            Line::from("  e           Edit rule"),
            Line::from("  d           Delete rule"),
            Line::from(""),
            Line::from("Press any key to close"),
        ];
        let popup = Paragraph::new(help)
            .block(Block::default().borders(Borders::ALL).title(" Help "))
            .wrap(Wrap { trim: false });
        frame.render_widget(popup, area);
    }

    // ─── Edit Popup ──────────────────────────────────────────────────────────

    fn render_edit_popup(&self, frame: &mut Frame, idx: usize) {
        if idx >= self.config_items.len() {
            return;
        }
        let (group, key, _) = &self.config_items[idx];
        let area = centered_rect(50, 20, frame.area());
        frame.render_widget(Clear, area);

        let text = vec![
            Line::from(format!("Editing: [{}] {}", group, key)),
            Line::from(""),
            Line::from(format!("> {}_", self.config_edit_buf)),
            Line::from(""),
            Line::from("Enter: save | Esc: cancel"),
        ];
        let popup = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Edit Value "))
            .wrap(Wrap { trim: false });
        frame.render_widget(popup, area);
    }

    // ─── Help ─────────────────────────────────────────────────────────────────

    fn render_help(&self, frame: &mut Frame, area: Rect) {
        let version = env!("CARGO_PKG_VERSION");

        let mut lines = vec![
            Line::from(Span::styled(
                "Fry TFTP Server",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "High-performance, cross-platform TFTP server",
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(format!("Version: {}", version)),
            Line::from(""),
            Line::from(Span::styled(
                "━━━ Supported RFCs ━━━",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("  RFC 1350  ", Style::default().fg(Color::Cyan)),
                Span::raw("TFTP Protocol — RRQ, WRQ, DATA, ACK, ERROR, octet & netascii modes"),
            ]),
            Line::from(vec![
                Span::styled("  RFC 2347  ", Style::default().fg(Color::Cyan)),
                Span::raw("Option Extension — OACK negotiation for extended options"),
            ]),
            Line::from(vec![
                Span::styled("  RFC 2348  ", Style::default().fg(Color::Cyan)),
                Span::raw("Blocksize Option — configurable block size (8–65464 bytes)"),
            ]),
            Line::from(vec![
                Span::styled("  RFC 2349  ", Style::default().fg(Color::Cyan)),
                Span::raw("Timeout & Transfer Size — timeout negotiation and tsize reporting"),
            ]),
            Line::from(vec![
                Span::styled("  RFC 7440  ", Style::default().fg(Color::Cyan)),
                Span::raw("Windowsize Option — sliding window for high throughput"),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "━━━ Features ━━━",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

        let features = [
            "GUI mode (egui) with dashboard, file browser, transfer history, log viewer",
            "TUI mode (ratatui) for terminal-based operation",
            "Headless mode for server/daemon deployment",
            "Hot-reload configuration via file watcher and SIGHUP",
            "Access Control Lists (ACL) with whitelist/blacklist and CIDR support",
            "Per-IP rate limiting and session limits",
            "Memory-mapped file I/O for large file transfers",
            "Sliding window protocol for high throughput (500+ MB/s)",
            "Netascii and octet transfer modes",
            "Path traversal protection and symlink policy enforcement",
            "Circular log rotation with configurable line limits",
            "System tray integration with status indicators",
            "Windows Service, systemd, and launchd support",
            "Environment variable overrides (TFTP_SERVER_*)",
        ];
        for feat in &features {
            lines.push(Line::from(vec![
                Span::styled("  - ", Style::default().fg(Color::Green)),
                Span::raw(*feat),
            ]));
        }

        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "━━━ About ━━━",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Author:  ", Style::default().fg(Color::Cyan)),
                Span::raw("Viacheslav Gordeev"),
            ]),
            Line::from(vec![
                Span::styled("  Email:   ", Style::default().fg(Color::Cyan)),
                Span::raw("qulisun@gmail.com"),
            ]),
            Line::from(vec![
                Span::styled("  Source:  ", Style::default().fg(Color::Cyan)),
                Span::raw("github.com/qulisun/fry-tftp-server"),
            ]),
            Line::from(vec![
                Span::styled("  License: ", Style::default().fg(Color::Cyan)),
                Span::raw("Proprietary"),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Built with Rust, egui, ratatui, tokio",
                Style::default().fg(Color::DarkGray),
            )),
        ]);

        let help = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Help "))
            .wrap(Wrap { trim: false });
        frame.render_widget(help, area);
    }
}

/// Create a centered rect with percentage width/height
fn centered_rect(pct_x: u16, pct_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(LayoutDirection::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(LayoutDirection::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
