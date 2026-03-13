use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// A single captured log entry
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: std::time::SystemTime,
    pub level: tracing::Level,
    pub target: String,
    pub message: String,
}

/// Shared log buffer type used by both GUI and TUI
pub type LogBuffer = Arc<Mutex<VecDeque<LogEntry>>>;

/// Tracing layer that captures log events into a shared buffer
pub struct AppLogLayer {
    buffer: LogBuffer,
    max_entries: usize,
}

impl AppLogLayer {
    pub fn new() -> (Self, LogBuffer) {
        let buffer = Arc::new(Mutex::new(VecDeque::with_capacity(10_000)));
        let layer = Self {
            buffer: buffer.clone(),
            max_entries: 10_000,
        };
        (layer, buffer)
    }
}

struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else {
            if !self.message.is_empty() {
                self.message.push(' ');
            }
            write!(&mut self.message, "{}={:?}", field.name(), value).ok();
        }
    }
}

impl<S> Layer<S> for AppLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor {
            message: String::new(),
        };
        event.record(&mut visitor);

        let entry = LogEntry {
            timestamp: std::time::SystemTime::now(),
            level: *event.metadata().level(),
            target: event.metadata().target().to_string(),
            message: visitor.message,
        };

        if let Ok(mut buf) = self.buffer.lock() {
            buf.push_back(entry);
            while buf.len() > self.max_entries {
                buf.pop_front();
            }
        }
    }
}

/// Load the last `max_lines` from a log file into the LogBuffer.
/// Parses tracing-subscriber's default format: `2026-03-13T10:00:00Z  INFO target: message`
/// Lines that don't match are loaded as INFO with raw text.
pub fn load_logs_from_file(buffer: &LogBuffer, path: &Path, max_lines: usize) {
    // Try to find the most recent log file (tracing-appender daily adds date suffix)
    // First try the exact path, then look for dated files in same directory
    let candidates = find_log_files(path);

    for candidate in candidates {
        if let Ok(file) = std::fs::File::open(&candidate) {
            let reader = std::io::BufReader::new(file);
            let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

            // Take only the last max_lines
            let start = lines.len().saturating_sub(max_lines);
            let tail = &lines[start..];

            if let Ok(mut buf) = buffer.lock() {
                for line in tail {
                    let entry = parse_log_line(line);
                    buf.push_back(entry);
                }
            }
            return; // loaded from the most recent file
        }
    }
}

fn find_log_files(base_path: &Path) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();

    // tracing-appender daily creates files like "fry-tftp-server.log.2026-03-13"
    // Try to find dated files in the same directory, sorted descending (most recent first)
    if let Some(parent) = base_path.parent() {
        if let Some(filename) = base_path.file_name().and_then(|f| f.to_str()) {
            if let Ok(entries) = std::fs::read_dir(parent) {
                let mut dated: Vec<std::path::PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|f| f.to_str())
                            .is_some_and(|f| f.starts_with(filename) && f.len() > filename.len())
                    })
                    .collect();
                dated.sort();
                dated.reverse(); // most recent first
                results.extend(dated);
            }
        }
    }

    // Also try the exact path (in case log_file is a plain file, not rolling)
    if base_path.exists() {
        results.push(base_path.to_path_buf());
    }

    results
}

fn parse_log_line(line: &str) -> LogEntry {
    // Format: "  2026-03-13T10:00:00.123Z  INFO target: message"
    // or:     "2026-03-13T10:00:00.123Z  INFO target: message"
    let trimmed = line.trim();

    let level = if trimmed.contains(" ERROR ") {
        tracing::Level::ERROR
    } else if trimmed.contains(" WARN ") {
        tracing::Level::WARN
    } else if trimmed.contains(" DEBUG ") {
        tracing::Level::DEBUG
    } else if trimmed.contains(" TRACE ") {
        tracing::Level::TRACE
    } else {
        tracing::Level::INFO
    };

    // Try to extract target and message after level keyword
    let level_str = match level {
        tracing::Level::ERROR => " ERROR ",
        tracing::Level::WARN => " WARN ",
        tracing::Level::DEBUG => " DEBUG ",
        tracing::Level::TRACE => " TRACE ",
        _ => " INFO ",
    };

    let (target, message) = if let Some(pos) = trimmed.find(level_str) {
        let after = &trimmed[pos + level_str.len()..];
        if let Some(colon_pos) = after.find(": ") {
            (
                after[..colon_pos].trim().to_string(),
                after[colon_pos + 2..].to_string(),
            )
        } else {
            ("app".to_string(), after.to_string())
        }
    } else {
        ("app".to_string(), trimmed.to_string())
    };

    LogEntry {
        timestamp: std::time::SystemTime::now(), // approximate — original timestamp lost
        level,
        target,
        message,
    }
}

/// Truncate a log file to keep only the last `max_lines` lines (circular rotation).
/// Also truncates any dated rolling log files in the same directory.
/// Call this periodically or at startup.
pub fn truncate_log_file(path: &Path, max_lines: usize) {
    if max_lines == 0 {
        return; // unlimited
    }

    let candidates = find_log_files(path);
    for file_path in candidates {
        truncate_single_file(&file_path, max_lines);
    }
}

fn truncate_single_file(path: &PathBuf, max_lines: usize) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= max_lines {
        return; // already within limit
    }

    // Keep only the last max_lines
    let start = lines.len() - max_lines;
    let truncated: String = lines[start..].iter().map(|l| format!("{}\n", l)).collect();

    if let Err(e) = std::fs::write(path, truncated) {
        tracing::warn!(path=%path.display(), error=%e, "failed to truncate log file");
    }
}
