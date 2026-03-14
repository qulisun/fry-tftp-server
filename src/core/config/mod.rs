use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Captured CLI overrides so they survive config reload.
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub config_path: Option<PathBuf>,
    pub port: Option<u16>,
    pub bind: Option<String>,
    pub root: Option<PathBuf>,
    pub allow_write: bool,
    pub max_sessions: Option<usize>,
    pub blksize: Option<u16>,
    pub windowsize: Option<u16>,
    pub ip_version: Option<String>,
    pub log_level: Option<String>,
}

/// Log a message via tracing if initialized, otherwise fall back to eprintln.
/// This is needed because Config::load() is called both at startup (before tracing)
/// and at runtime (e.g. server restart from TUI/GUI where tracing is active).
fn log_or_eprint(msg: String) {
    use tracing::dispatcher;
    dispatcher::get_default(|d| {
        if d.is::<tracing::subscriber::NoSubscriber>() {
            eprintln!("{}", msg);
        } else {
            tracing::info!("{}", msg);
        }
    });
}

/// Full server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub server: ServerConfig,
    pub network: NetworkConfig,
    pub protocol: ProtocolConfig,
    pub session: SessionConfig,
    pub security: SecurityConfig,
    pub filesystem: FilesystemConfig,
    pub acl: AclConfig,
    #[serde(default)]
    pub gui: GuiConfig,
    #[serde(default)]
    pub tui: TuiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiConfig {
    pub theme: String,
    pub refresh_rate_ms: u64,
    pub graph_history_seconds: u64,
    pub show_bandwidth_chart: bool,
    pub language: String,
}

/// Detect system language from OS locale. Returns language code if supported, "en" otherwise.
/// Checks LANG/LC_ALL env vars (Linux/macOS) and Win32 API locale (Windows).
fn detect_system_language() -> String {
    let locale = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .unwrap_or_default()
        .to_lowercase();

    // locale is typically "en_US.UTF-8", "ru_RU.UTF-8", "de_DE.UTF-8" etc.
    let code = if locale.starts_with("ru") {
        "ru"
    } else if locale.starts_with("de") {
        "de"
    } else if locale.starts_with("es") {
        "es"
    } else if locale.starts_with("fr") {
        "fr"
    } else {
        "en"
    };
    code.to_string()
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            refresh_rate_ms: 250,
            graph_history_seconds: 300,
            show_bandwidth_chart: false,
            language: detect_system_language(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    pub mouse: bool,
    pub refresh_rate_ms: u64,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            mouse: true,
            refresh_rate_ms: 250,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind_address: String,
    pub port: u16,
    pub root: PathBuf,
    pub log_level: String,
    pub log_file: String,
    /// Maximum lines to keep in the log file (circular rotation). 0 = unlimited.
    pub max_log_lines: usize,
    /// Enable system log integration (journald on Linux, Event Log on Windows)
    pub syslog: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub ip_version: String,
    pub recv_buffer_size: usize,
    pub send_buffer_size: usize,
    pub session_recv_buffer: usize,
    pub session_send_buffer: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtocolConfig {
    pub allow_write: bool,
    pub default_blksize: u16,
    pub max_blksize: u16,
    pub default_windowsize: u16,
    pub max_windowsize: u16,
    pub default_timeout: u8,
    pub min_timeout: u8,
    pub max_timeout: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    pub max_sessions: usize,
    pub max_retries: u32,
    pub exponential_backoff: bool,
    pub session_timeout: u64,
    pub shutdown_grace_period: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    pub per_ip_max_sessions: usize,
    pub per_ip_rate_limit: u32,
    pub rate_limit_window_seconds: u64,
    pub rate_limit_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FilesystemConfig {
    pub max_file_size: String,
    pub allow_overwrite: bool,
    pub create_dirs: bool,
    pub follow_symlinks: bool,
    #[serde(default)]
    pub virtual_roots: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AclConfig {
    pub mode: String,
    pub rules: Vec<AclRuleConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRuleConfig {
    pub action: String,
    pub source: String,
    pub operations: Vec<String>,
    #[serde(default)]
    pub comment: String,
}

// Platform-specific defaults
fn default_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        PathBuf::from(r"C:\TFTP")
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .map(|h| h.join("Library/TFTP"))
            .unwrap_or_else(|| PathBuf::from("/tmp/tftp"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        PathBuf::from("/srv/tftp")
    }
}

fn default_log_file() -> String {
    #[cfg(target_os = "windows")]
    {
        dirs::data_dir()
            .map(|d| {
                d.join("fry-tftp-server")
                    .join("fry-tftp-server.log")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_default()
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .map(|h| {
                h.join("Library/Logs/fry-tftp-server.log")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_default()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "/var/log/fry-tftp-server.log".to_string()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "::".to_string(),
            port: 69,
            root: default_root(),
            log_level: "info".to_string(),
            log_file: default_log_file(),
            max_log_lines: 5000,
            syslog: false,
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            ip_version: "dual".to_string(),
            recv_buffer_size: 4 * 1024 * 1024,
            send_buffer_size: 4 * 1024 * 1024,
            session_recv_buffer: 2 * 1024 * 1024,
            session_send_buffer: 2 * 1024 * 1024,
        }
    }
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            allow_write: false,
            default_blksize: 512,
            max_blksize: 65464,
            default_windowsize: 1,
            max_windowsize: 64,
            default_timeout: 3,
            min_timeout: 1,
            max_timeout: 255,
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_sessions: 100,
            max_retries: 5,
            exponential_backoff: true,
            session_timeout: 120,
            shutdown_grace_period: 30,
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            per_ip_max_sessions: 10,
            per_ip_rate_limit: 100,
            rate_limit_window_seconds: 60,
            rate_limit_action: "drop".to_string(),
        }
    }
}

impl Default for FilesystemConfig {
    fn default() -> Self {
        Self {
            max_file_size: "4GB".to_string(),
            allow_overwrite: false,
            create_dirs: false,
            follow_symlinks: false,
            virtual_roots: std::collections::HashMap::new(),
        }
    }
}

impl Default for AclConfig {
    fn default() -> Self {
        Self {
            mode: "disabled".to_string(),
            rules: Vec::new(),
        }
    }
}

/// Parse a human-readable size string like "4GB", "100MB", "512KB", "1024" into bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, unit) = if s.ends_with("GB") || s.ends_with("gb") {
        (&s[..s.len() - 2], 1024u64 * 1024 * 1024)
    } else if s.ends_with("MB") || s.ends_with("mb") {
        (&s[..s.len() - 2], 1024u64 * 1024)
    } else if s.ends_with("KB") || s.ends_with("kb") {
        (&s[..s.len() - 2], 1024u64)
    } else if s.ends_with('B') || s.ends_with('b') {
        (&s[..s.len() - 1], 1u64)
    } else {
        (s, 1u64) // plain number = bytes
    };
    num_part
        .trim()
        .parse::<u64>()
        .ok()
        .map(|n| n.saturating_mul(unit))
}

impl FilesystemConfig {
    /// Parse max_file_size string into bytes. Returns u64::MAX on parse failure.
    pub fn max_file_size_bytes(&self) -> u64 {
        parse_size(&self.max_file_size).unwrap_or(u64::MAX)
    }
}

impl Config {
    /// Load config from a TOML file, falling back to defaults
    pub fn load(path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        if let Some(path) = path {
            if path.exists() {
                let content = std::fs::read_to_string(path)?;
                let mut config: Config = toml::from_str(&content)?;
                config.apply_env_overrides();
                return Ok(config);
            }
        }

        // Try platform-specific default paths
        for candidate in Self::default_config_paths() {
            if candidate.exists() {
                let content = std::fs::read_to_string(&candidate)?;
                let mut config: Config = toml::from_str(&content)?;
                config.apply_env_overrides();
                log_or_eprint(format!("[config] loaded from {}", candidate.display()));
                return Ok(config);
            }
        }

        log_or_eprint("[config] no config file found, using defaults".to_string());
        let mut config = Config::default();
        config.apply_env_overrides();
        // Auto-create config file with defaults so GUI/TUI changes persist on next restart
        match config.save() {
            Ok(path) => log_or_eprint(format!(
                "[config] created default config at {}",
                path.display()
            )),
            Err(e) => log_or_eprint(format!("[config] could not create default config: {}", e)),
        }
        Ok(config)
    }

    /// Save config to the first writable default config path.
    /// Creates parent directories if needed.
    pub fn save(&self) -> anyhow::Result<PathBuf> {
        let paths = Self::default_config_paths();
        let target = paths
            .first()
            .ok_or_else(|| anyhow::anyhow!("no config path available"))?
            .clone();

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(&target, &content)?;
        tracing::info!(path = %target.display(), "config saved to disk");
        Ok(target)
    }

    /// Return the first existing config file path (used for file watching)
    pub fn config_file_path() -> Option<PathBuf> {
        Self::default_config_paths()
            .into_iter()
            .find(|p| p.exists())
    }

    fn default_config_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        #[cfg(target_os = "windows")]
        {
            if let Some(appdata) = dirs::data_dir() {
                paths.push(appdata.join("fry-tftp-server").join("config.toml"));
            }
        }

        #[cfg(target_os = "macos")]
        {
            if let Some(home) = dirs::home_dir() {
                paths.push(home.join("Library/Preferences/fry-tftp-server/config.toml"));
            }
        }

        #[cfg(target_os = "linux")]
        {
            // User-writable path first (for save), system path as fallback (for load)
            if let Some(home) = dirs::home_dir() {
                paths.push(home.join(".config/fry-tftp-server/config.toml"));
            }
            paths.push(PathBuf::from("/etc/fry-tftp-server/config.toml"));
        }

        paths.push(PathBuf::from("config.toml"));
        paths
    }

    /// Apply environment variable overrides (priority: CLI > env > file > defaults)
    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("TFTP_SERVER_PORT") {
            if let Ok(p) = v.parse() {
                self.server.port = p;
            }
        }
        if let Ok(v) = std::env::var("TFTP_SERVER_BIND_ADDRESS") {
            self.server.bind_address = v;
        }
        if let Ok(v) = std::env::var("TFTP_SERVER_ROOT") {
            self.server.root = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("TFTP_SERVER_LOG_LEVEL") {
            self.server.log_level = v;
        }
        if let Ok(v) = std::env::var("TFTP_SERVER_LOG_FILE") {
            self.server.log_file = v;
        }
        if let Ok(v) = std::env::var("TFTP_SERVER_ALLOW_WRITE") {
            self.protocol.allow_write = v == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("TFTP_SERVER_MAX_SESSIONS") {
            if let Ok(n) = v.parse() {
                self.session.max_sessions = n;
            }
        }
        if let Ok(v) = std::env::var("TFTP_SERVER_IP_VERSION") {
            self.network.ip_version = v;
        }
    }

    /// Apply CLI overrides
    #[allow(clippy::too_many_arguments)]
    pub fn apply_overrides(
        &mut self,
        port: Option<u16>,
        bind: Option<String>,
        root: Option<PathBuf>,
        allow_write: bool,
        max_sessions: Option<usize>,
        blksize: Option<u16>,
        windowsize: Option<u16>,
        ip_version: Option<String>,
        log_level: Option<String>,
    ) {
        if let Some(p) = port {
            self.server.port = p;
        }
        if let Some(b) = bind {
            self.server.bind_address = b;
        }
        if let Some(r) = root {
            self.server.root = r;
        }
        if allow_write {
            self.protocol.allow_write = true;
        }
        if let Some(m) = max_sessions {
            self.session.max_sessions = m;
        }
        if let Some(b) = blksize {
            self.protocol.max_blksize = b;
        }
        if let Some(w) = windowsize {
            self.protocol.max_windowsize = w;
        }
        if let Some(v) = ip_version {
            self.network.ip_version = v;
        }
        if let Some(l) = log_level {
            self.server.log_level = l;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_gb() {
        assert_eq!(parse_size("4GB"), Some(4 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("1gb"), Some(1024 * 1024 * 1024));
    }

    #[test]
    fn test_parse_size_mb() {
        assert_eq!(parse_size("100MB"), Some(100 * 1024 * 1024));
        assert_eq!(parse_size("256mb"), Some(256 * 1024 * 1024));
    }

    #[test]
    fn test_parse_size_kb() {
        assert_eq!(parse_size("512KB"), Some(512 * 1024));
    }

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size("1024B"), Some(1024));
        assert_eq!(parse_size("1024"), Some(1024));
    }

    #[test]
    fn test_parse_size_invalid() {
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size(""), None);
    }

    #[test]
    fn test_parse_size_with_spaces() {
        assert_eq!(parse_size(" 4 GB"), Some(4 * 1024 * 1024 * 1024));
    }

    #[test]
    fn test_default_config_roundtrip() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.server.port, config.server.port);
        assert_eq!(parsed.protocol.allow_write, config.protocol.allow_write);
    }

    #[test]
    fn test_max_file_size_bytes() {
        let fs = FilesystemConfig {
            max_file_size: "4GB".to_string(),
            ..Default::default()
        };
        assert_eq!(fs.max_file_size_bytes(), 4 * 1024 * 1024 * 1024);
    }
}
