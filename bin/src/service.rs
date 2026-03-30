//! Windows service management module.
//!
//! This module provides functionality to run the application as a Windows service,
//! including service registration, start/stop handling, and graceful restart support.
//!
//! Command-line arguments can be specified when installing the service:
//! ```cmd
//! ferron winservice install --config config.toml --config-params "key=value"
//! ```

#![cfg_attr(not(windows), allow(dead_code, unused_imports, unused_variables))]

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

#[cfg(windows)]
use windows_service::{
    define_windows_service,
    service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceStatus, ServiceType},
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

/// Service name for Windows service registration
pub const SERVICE_NAME: &str = "Ferron";

/// Service display name
pub const SERVICE_DISPLAY_NAME: &str = "Ferron web server";

/// Service description
pub const SERVICE_DESCRIPTION: &str =
    "Ferron web server with graceful shutdown and config reload support"; // TODO: replace with actual description

pub static PARAMS: OnceLock<(Option<String>, Option<String>, Option<String>, bool)> =
    OnceLock::new();

/// Entry point for the Windows service
#[cfg(windows)]
pub fn run_service(
    config_path: Option<String>,
    config_params: Option<String>,
    config_adapter: Option<String>,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = PARAMS.set((config_path, config_params, config_adapter, verbose));

    // Define the service entry point
    define_windows_service!(ffi_service_entry, service_main);

    // Start the service dispatcher
    service_dispatcher::start(SERVICE_NAME, ffi_service_entry)?;

    Ok(())
}

/// Main service function called by the service dispatcher
/// Receives arguments from the service control manager
#[cfg(windows)]
fn service_main(_arguments: Vec<std::ffi::OsString>) {
    if let Err(e) = run_service_impl() {
        eprintln!("Service failed: {}", e);
    }
}

#[cfg(windows)]
fn run_service_impl() -> Result<(), Box<dyn std::error::Error>> {
    use windows_service::service::ServiceState;

    // Register service control handler
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop => {
                ferron_core::log_debug!("Service stop signal received");
                ferron_core::shutdown::SHUTDOWN_TOKEN
                    .swap(Arc::new(tokio_util::sync::CancellationToken::new()))
                    .cancel();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::ParamChange => {
                // Treat parameter change as a config reload signal
                ferron_core::log_debug!("Service config reload signal received (ParamChange)");
                ferron_core::shutdown::RELOAD_TOKEN
                    .swap(Arc::new(tokio_util::sync::CancellationToken::new()))
                    .cancel();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    let Some((config_path, config_params, config_adapter, verbose)) = PARAMS.get().cloned() else {
        Err(anyhow::anyhow!(
            "Windows service command line parameters not set"
        ))?
    };

    // Log the configuration
    ferron_core::log_info!("{} service started", SERVICE_NAME,);

    // Tell the service we're running
    let mut status = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::PARAM_CHANGE,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    status_handle.set_service_status(status.clone())?;

    // Run the application with the parsed configuration
    let server_result = crate::run(config_path, config_params, config_adapter, verbose, false);

    if let Err(e) = server_result {
        ferron_core::log_error!("Server error: {}", e);
    }

    // Update service status to stopped
    status.current_state = ServiceState::Stopped;
    status_handle.set_service_status(status)?;

    ferron_core::log_info!("{} service stopped", SERVICE_NAME);

    Ok(())
}

fn parse_config_params(params_str: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for pair in params_str.split(';') {
        if let Some((key, value)) = pair.split_once('=') {
            params.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    params
}

use ferron_core::registry::RegistryBuilder;

/// Check if running as a Windows service
#[cfg(windows)]
pub fn is_running_as_service() -> bool {
    std::env::var("WINDOWS_SERVICE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// On non-Windows platforms, service support is not available
#[cfg(not(windows))]
pub fn run_service() -> Result<(), Box<dyn std::error::Error>> {
    Err("Windows services are not supported on this platform".into())
}

#[cfg(not(windows))]
pub fn is_running_as_service() -> bool {
    false
}

/// Install the service on Windows with optional command-line arguments
#[cfg(windows)]
pub fn install_service(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    use windows_service::{
        service::{ServiceAccess, ServiceInfo, ServiceStartType},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let exe_path = std::env::current_exe()?;

    // Build launch arguments: always include --service flag plus any user-provided args
    let mut launch_arguments: Vec<std::ffi::OsString> = vec!["run".into(), "--service".into()];
    launch_arguments.extend(args.into_iter().map(|s| s.into()));

    let service_info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: SERVICE_DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: windows_service::service::ServiceErrorControl::Normal,
        executable_path: exe_path,
        launch_arguments,
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };

    let service = service_manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG)?;

    service.set_description(SERVICE_DESCRIPTION)?;

    println!("Service '{}' installed successfully", SERVICE_NAME);
    println!("The service will start with the configured arguments.");
    println!();
    println!("To start the service:");
    println!("  sc start {}", SERVICE_NAME);
    println!();
    println!("To view service logs, open Event Viewer and navigate to:");
    println!(
        "  Windows Logs -> Application (look for '{}' source)",
        SERVICE_NAME
    );

    Ok(())
}

/// Uninstall the service on Windows
#[cfg(windows)]
pub fn uninstall_service() -> Result<(), Box<dyn std::error::Error>> {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let service_access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
    let service = service_manager.open_service(SERVICE_NAME, service_access)?;

    // Stop the service if running
    if let Ok(status) = service.query_status() {
        if status.current_state != windows_service::service::ServiceState::Stopped {
            println!("Stopping service...");
            service.stop()?;

            // Wait for service to stop
            let timeout = Duration::from_secs(30);
            let start = std::time::Instant::now();
            while start.elapsed() < timeout {
                std::thread::sleep(Duration::from_secs(1));
                if let Ok(status) = service.query_status() {
                    if status.current_state == windows_service::service::ServiceState::Stopped {
                        break;
                    }
                }
            }
        }
    }

    service.delete()?;

    println!("Service '{}' uninstalled successfully", SERVICE_NAME);

    Ok(())
}

#[cfg(not(windows))]
pub fn install_service(_args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    Err("Service installation is only supported on Windows".into())
}

#[cfg(not(windows))]
pub fn uninstall_service() -> Result<(), Box<dyn std::error::Error>> {
    Err("Service uninstallation is only supported on Windows".into())
}
