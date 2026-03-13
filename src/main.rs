use clap::Parser;
use std::path::PathBuf;

use fry_tftp_server::core::config::Config;
use fry_tftp_server::core::state::AppState;

#[derive(Parser, Debug)]
#[command(
    name = "fry-tftp-server",
    about = "Fry TFTP Server — cross-platform high-performance TFTP server",
    version
)]
struct Cli {
    /// Run in GUI mode (default when gui feature is enabled)
    #[arg(long)]
    gui: bool,

    /// Run in TUI mode
    #[arg(long)]
    tui: bool,

    /// Run in headless mode (daemon)
    #[arg(long)]
    headless: bool,

    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Root directory (overrides config)
    #[arg(short, long)]
    root: Option<PathBuf>,

    /// Port number (overrides config)
    #[arg(short, long)]
    port: Option<u16>,

    /// Bind address (overrides config)
    #[arg(short, long)]
    bind: Option<String>,

    /// Allow write requests (overrides config)
    #[arg(long)]
    allow_write: bool,

    /// Maximum parallel sessions
    #[arg(long)]
    max_sessions: Option<usize>,

    /// Maximum block size
    #[arg(long)]
    blksize: Option<u16>,

    /// Maximum window size
    #[arg(long)]
    windowsize: Option<u16>,

    /// IP version: dual | v4 | v6
    #[arg(long)]
    ip_version: Option<String>,

    /// Increase verbosity (-v info, -vv debug, -vvv trace)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Quiet mode (errors only)
    #[arg(short, long)]
    quiet: bool,

    /// Install as Windows service
    #[cfg(windows)]
    #[arg(long)]
    install_service: bool,

    /// Uninstall Windows service
    #[cfg(windows)]
    #[arg(long)]
    uninstall_service: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Windows service install/uninstall
    #[cfg(windows)]
    {
        if cli.install_service {
            return install_windows_service();
        }
        if cli.uninstall_service {
            return uninstall_windows_service();
        }
    }

    // Load config
    let mut config = Config::load(cli.config.as_deref())?;

    // Determine log level from CLI
    let log_level = if cli.quiet {
        Some("error".to_string())
    } else {
        match cli.verbose {
            0 => None,
            1 => Some("info".to_string()),
            2 => Some("debug".to_string()),
            _ => Some("trace".to_string()),
        }
    };

    // Capture CLI overrides for config reload
    let cli_overrides = fry_tftp_server::core::config::CliOverrides {
        config_path: cli.config.map(PathBuf::from),
        port: cli.port,
        bind: cli.bind.clone(),
        root: cli.root.clone(),
        allow_write: cli.allow_write,
        max_sessions: cli.max_sessions,
        blksize: cli.blksize,
        windowsize: cli.windowsize,
        ip_version: cli.ip_version.clone(),
        log_level: log_level.clone(),
    };

    // Apply CLI overrides
    config.apply_overrides(
        cli_overrides.port,
        cli_overrides.bind.clone(),
        cli_overrides.root.clone(),
        cli_overrides.allow_write,
        cli_overrides.max_sessions,
        cli_overrides.blksize,
        cli_overrides.windowsize,
        cli_overrides.ip_version.clone(),
        cli_overrides.log_level.clone(),
    );

    // Resource limits check (E2, Unix only)
    #[cfg(unix)]
    check_resource_limits();

    // Ensure root directory exists
    if !config.server.root.exists() {
        std::fs::create_dir_all(&config.server.root)?;
    }

    // Build tokio runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Determine mode and run
    if cli.gui {
        #[cfg(feature = "gui")]
        {
            let log_buffer = init_logging_with_buffer(&config, true);
            let state = AppState::new(config, cli_overrides.clone());
            runtime.block_on(fry_tftp_server::gui::run(state, log_buffer))?;
        }
        #[cfg(not(feature = "gui"))]
        {
            anyhow::bail!("GUI feature not compiled. Build with: cargo build --features gui");
        }
    } else if cli.tui {
        #[cfg(feature = "tui")]
        {
            let log_buffer = init_logging_with_buffer(&config, false);
            let state = AppState::new(config, cli_overrides.clone());
            runtime.block_on(fry_tftp_server::tui::run(state, log_buffer))?;
        }
        #[cfg(not(feature = "tui"))]
        {
            anyhow::bail!("TUI feature not compiled. Build with: cargo build --features tui");
        }
    } else {
        init_logging(&config);
        let state = AppState::new(config, cli_overrides);
        runtime.block_on(fry_tftp_server::headless::run(state))?;
    }

    Ok(())
}

fn init_logging(config: &Config) {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};

    let filter =
        EnvFilter::try_new(&config.server.log_level).unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false);

    // Optional file layer
    let file_layer = if !config.server.log_file.is_empty() {
        let log_path = PathBuf::from(&config.server.log_file);
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let file_appender = tracing_appender::rolling::daily(
            log_path.parent().unwrap_or(std::path::Path::new(".")),
            log_path
                .file_name()
                .unwrap_or(std::ffi::OsStr::new("fry-tftp-server.log")),
        );
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        std::mem::forget(_guard);

        Some(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false)
                .with_ansi(false)
                .with_writer(non_blocking),
        )
    } else {
        None
    };

    // Optional system log layer (journald on Linux, Event Log on Windows)
    let syslog_layer = if config.server.syslog {
        init_syslog_layer()
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(file_layer)
        .with(syslog_layer)
        .init();
}

/// Create a system logging layer (journald on Unix, Event Log on Windows).
/// Returns None if initialization fails (e.g., journald not available).
#[cfg(unix)]
fn init_syslog_layer() -> Option<tracing_journald::Layer> {
    match tracing_journald::layer() {
        Ok(layer) => {
            eprintln!("[logging] journald syslog layer enabled");
            Some(layer)
        }
        Err(e) => {
            eprintln!(
                "[logging] failed to connect to journald: {} (syslog disabled)",
                e
            );
            None
        }
    }
}

/// Create a Windows Event Log layer.
/// Returns None if initialization fails.
#[cfg(windows)]
fn init_syslog_layer() -> Option<fry_tftp_server::platform::windows_eventlog::EventLogLayer> {
    match fry_tftp_server::platform::windows_eventlog::EventLogLayer::new("Fry TFTP Server") {
        Ok(layer) => {
            eprintln!("[logging] Windows Event Log layer enabled");
            Some(layer)
        }
        Err(e) => {
            eprintln!(
                "[logging] failed to open Windows Event Log: {} (syslog disabled)",
                e
            );
            None
        }
    }
}

#[cfg(any(feature = "gui", feature = "tui"))]
/// Initialize logging with an in-app log buffer.
/// When `console_output` is false (TUI mode), the console fmt layer is suppressed
/// to prevent raw text from corrupting the terminal UI.
fn init_logging_with_buffer(
    config: &Config,
    console_output: bool,
) -> fry_tftp_server::core::log_buffer::LogBuffer {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};

    let filter =
        EnvFilter::try_new(&config.server.log_level).unwrap_or_else(|_| EnvFilter::new("info"));

    let (app_layer, log_buffer) = fry_tftp_server::core::log_buffer::AppLogLayer::new();

    // Load previous log entries from file into the buffer
    let log_file_path = &config.server.log_file;
    if !log_file_path.is_empty() {
        fry_tftp_server::core::log_buffer::load_logs_from_file(
            &log_buffer,
            std::path::Path::new(log_file_path),
            500,
        );
    }

    // Optional file logging layer
    let file_layer = if !log_file_path.is_empty() {
        let log_path = PathBuf::from(log_file_path);
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file_appender = tracing_appender::rolling::daily(
            log_path.parent().unwrap_or(std::path::Path::new(".")),
            log_path
                .file_name()
                .unwrap_or(std::ffi::OsStr::new("fry-tftp-server.log")),
        );
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        std::mem::forget(_guard);

        Some(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false)
                .with_ansi(false)
                .with_writer(non_blocking),
        )
    } else {
        None
    };

    // Console layer — only for GUI, NOT for TUI (would corrupt terminal)
    let console_layer = if console_output {
        Some(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false),
        )
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(file_layer)
        .with(app_layer)
        .init();

    log_buffer
}

/// Install the TFTP server as a Windows service using sc.exe
#[cfg(windows)]
fn install_windows_service() -> anyhow::Result<()> {
    let exe_path = std::env::current_exe()?;
    let output = std::process::Command::new("sc.exe")
        .args([
            "create",
            "FryTFTPServer",
            &format!("binPath={} --headless", exe_path.display()),
            "start=auto",
            "DisplayName=Fry TFTP Server",
        ])
        .output()?;

    if output.status.success() {
        println!("Service 'FryTFTPServer' installed successfully.");
        println!("Start with: sc.exe start FryTFTPServer");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to install service: {}", stderr.trim());
    }
    Ok(())
}

/// Uninstall the TFTP server Windows service
#[cfg(windows)]
fn uninstall_windows_service() -> anyhow::Result<()> {
    let output = std::process::Command::new("sc.exe")
        .args(["delete", "FryTFTPServer"])
        .output()?;

    if output.status.success() {
        println!("Service 'FryTFTPServer' removed successfully.");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to remove service: {}", stderr.trim());
    }
    Ok(())
}

/// Check resource limits on Unix systems and warn if too low
#[cfg(unix)]
fn check_resource_limits() {
    use nix::sys::resource::{getrlimit, Resource};

    match getrlimit(Resource::RLIMIT_NOFILE) {
        Ok((soft, hard)) => {
            let recommended = 4096u64;
            if soft < recommended {
                eprintln!(
                    "[warning] File descriptor limit is low: soft={}, hard={} (recommended: {})",
                    soft, hard, recommended
                );
                eprintln!(
                    "[warning] Run 'ulimit -n {}' or adjust /etc/security/limits.conf",
                    recommended
                );
            }
        }
        Err(e) => {
            eprintln!("[warning] Could not check file descriptor limits: {}", e);
        }
    }
}
