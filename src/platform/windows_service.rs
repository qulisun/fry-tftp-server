/// Native Windows Service integration using `windows-service` crate.
///
/// Provides proper service_main entry point and service control handler
/// so the server can be managed via services.msc / sc.exe.
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use crate::core::config::Config;
use crate::core::state::AppState;

const SERVICE_NAME: &str = "FryTFTPServer";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

/// Attempt to run as a Windows Service.
/// Returns Ok(true) if successfully started as a service.
/// Returns Ok(false) if not running as a service (e.g., console mode).
/// Returns Err if there was a service-specific error.
pub fn try_run_as_service() -> Result<bool, windows_service::Error> {
    // service_dispatcher::start will fail if we're not actually running as a service
    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(()) => Ok(true),
        Err(windows_service::Error::Winapi(ref e)) if e.raw_os_error() == Some(1063) => {
            // ERROR_FAILED_SERVICE_CONTROLLER_CONNECT (1063)
            // This means we're running in console mode, not as a service
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(arguments: Vec<OsString>) {
    if let Err(e) = run_service(arguments) {
        tracing::error!(error=%e, "Windows service failed");
    }
}

fn run_service(_arguments: Vec<OsString>) -> Result<(), Box<dyn std::error::Error>> {
    let shutdown_token = CancellationToken::new();
    let reload_token = CancellationToken::new();

    // Set up service control handler
    let shutdown = shutdown_token.clone();
    let reload = reload_token.clone();
    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                tracing::info!("received service control: stop/shutdown");
                shutdown.cancel();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::ParamChange => {
                tracing::info!("received service control: param change (config reload)");
                reload.cancel();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })?;

    // Report Running status
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP
            | ServiceControlAccept::SHUTDOWN
            | ServiceControlAccept::PARAM_CHANGE,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    // Load config and run the server
    let config = Config::load(None)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = runtime.block_on(async {
        let state = AppState::new(config);
        // Wire shutdown token: when Windows service stops, cancel server
        let state_for_stop = state.clone();
        let svc_shutdown = shutdown_token.clone();
        tokio::spawn(async move {
            svc_shutdown.cancelled().await;
            state_for_stop.cancel_shutdown();
        });

        // Wire reload
        let reload_state = state.clone();
        tokio::spawn(async move {
            loop {
                reload_token.cancelled().await;
                tracing::info!("reloading config via service control");
                match Config::load(None) {
                    Ok(new_config) => {
                        reload_state.config.store(Arc::new(new_config));
                        tracing::info!("config reloaded successfully");
                    }
                    Err(e) => {
                        tracing::error!(error=%e, "failed to reload config");
                    }
                }
            }
        });

        crate::headless::run(state).await
    });

    // Report Stopped status
    let exit_code = if result.is_ok() {
        ServiceExitCode::Win32(0)
    } else {
        ServiceExitCode::Win32(1)
    };

    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    result.map_err(|e| e.into())
}
