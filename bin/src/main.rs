use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use clap::Parser;
use ferron_config_ferronconf::FerronConfConfigurationAdapterModuleLoader;
use ferron_config_json::JsonConfigurationAdapterModuleLoader;
use ferron_core::builtin::BuiltinModuleLoader;
use ferron_core::config::adapter::ConfigurationAdapter;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::loader::ModuleLoader;
use ferron_core::logging::LogLevel;
use ferron_core::registry::{Registry, RegistryBuilder};
use ferron_core::runtime::Runtime;
use ferron_core::shutdown::{RELOAD_TOKEN, SHUTDOWN_TOKEN};
use ferron_core::{log_debug, log_info, log_warn};
use ferron_http_server::BasicHttpModuleLoader;
use ferron_observability_format_json::JsonFormatObservabilityModuleLoader;
use malloc_best_effort::BEMalloc;

mod cli;
mod service;

#[cfg(unix)]
mod daemon;

use cli::{parse_config_params, Cli, Commands};

#[cfg(windows)]
use cli::WinServiceCommands;
use ferron_observability_consolelog::ConsoleObservabilityModuleLoader;
use ferron_tls_manual::TlsManualModuleLoader;

#[global_allocator]
static GLOBAL: BEMalloc = BEMalloc::new();

fn main() {
    BEMalloc::init();

    if let Err(e) = main_inner() {
        if !ferron_core::logging::is_init() {
            let _ = ferron_core::logging::init_stdio_logger(LogLevel::Error);
        }
        if ferron_core::logging::is_init() {
            ferron_core::log_error!("{}", e);
        } else {
            eprintln!("Error: {}", e);
        }
        std::process::exit(1);
    }
}

fn main_inner() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            config_path,
            config_params,
            config_adapter,
            verbose,
            #[cfg(windows)]
            service,
        } => {
            #[cfg(not(windows))]
            let service = false;
            run(config_path, config_params, config_adapter, verbose, service)?;
        }
        #[cfg(unix)]
        Commands::Daemon {
            config_path,
            config_params,
            config_adapter,
            verbose,
            pid_file,
        } => {
            run_daemon(
                config_path,
                config_params,
                config_adapter,
                verbose,
                pid_file,
            )?;
        }
        Commands::Validate {
            config_path,
            config_params,
            config_adapter,
        } => {
            validate(config_path, config_params, config_adapter)?;
        }
        Commands::Adapt {
            config_path,
            config_params,
            config_adapter,
        } => {
            adapt(config_path, config_params, config_adapter)?;
        }
        #[cfg(windows)]
        Commands::WinService { subcommand } => {
            winservice(subcommand)?;
        }
    }

    Ok(())
}

#[cfg(unix)]
fn run_daemon(
    config_path: Option<String>,
    config_params: Option<String>,
    config_adapter: Option<String>,
    verbose: bool,
    pid_file: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use ferron_core::log_info;
    use ferron_core::logging::LogLevel;

    // Initialize stdio logger for validation phase (before daemonizing)
    let log_level = if verbose {
        LogLevel::Debug
    } else {
        LogLevel::Info
    };

    if !ferron_core::logging::is_init() {
        ferron_core::logging::init_stdio_logger(log_level)?;
    }

    // First, validate the configuration before daemonizing
    log_info!("Validating configuration before daemonizing...");
    validate(
        config_path.clone(),
        config_params.clone(),
        config_adapter.clone(),
    )?;
    log_info!("Configuration validation successful");

    // Check if an existing daemon is already running (if PID file is specified)
    if let Some(ref pid_path) = pid_file {
        if daemon::check_pid_file(pid_path)? {
            return Err(
                format!("Daemon is already running (PID file exists: {})", pid_path).into(),
            );
        }
    }

    // Daemonize the process
    log_info!("Daemonizing process...");
    let is_daemon = daemon::daemonize()?;

    if !is_daemon {
        // This is the parent process, exit gracefully
        log_info!("Parent process exiting, daemon started in background");
        return Ok(());
    }

    // This is the daemon process

    // Re-initialize logger after daemonizing (stdout/stderr are now /dev/null)
    // For a daemon, we might want to log to syslog or a file, but for now we'll
    // keep the stdio logger (which will write to /dev/null)
    ferron_core::logging::init_stdio_logger(log_level)?;

    // Write PID file if specified
    if let Some(ref pid_path) = pid_file {
        daemon::write_pid_file(pid_path)?;

        // Set up cleanup on shutdown
        let pid_path = pid_path.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let shutdown_token = SHUTDOWN_TOKEN.load();
                shutdown_token.cancelled().await;
                let _ = daemon::remove_pid_file(&pid_path);
            });
        });
    }

    // Set up signal handlers
    daemon::setup_signal_handlers()?;
    log_info!("Signal handlers installed (SIGINT -> shutdown, SIGHUP -> reload)");

    // Now run the server with the same configuration
    log_info!("Starting web server as daemon...");
    run(config_path, config_params, config_adapter, verbose, false)?;

    Ok(())
}

#[cfg(windows)]
fn winservice(subcommand: WinServiceCommands) -> Result<(), Box<dyn std::error::Error>> {
    match subcommand {
        WinServiceCommands::Install {
            config_path,
            config_params,
            config_adapter,
            verbose,
        } => {
            // Build service arguments
            let mut args = Vec::new();

            if let Some(path) = config_path {
                args.push("--config".to_string());
                args.push(path);
            }

            if let Some(params) = config_params {
                args.push("--config-params".to_string());
                args.push(params);
            }

            if let Some(adapter) = config_adapter {
                args.push("--config-adapter".to_string());
                args.push(adapter);
            }

            if verbose {
                args.push("--verbose".to_string());
            }

            service::install_service(args)?;
        }
        WinServiceCommands::Uninstall => {
            service::uninstall_service()?;
        }
    }

    Ok(())
}

fn get_loaders() -> Vec<Box<dyn ModuleLoader>> {
    vec![
        Box::new(BuiltinModuleLoader),
        Box::new(BasicHttpModuleLoader::default()),
        Box::new(JsonConfigurationAdapterModuleLoader),
        Box::new(FerronConfConfigurationAdapterModuleLoader),
        Box::new(TlsManualModuleLoader),
        Box::new(ConsoleObservabilityModuleLoader::default()),
        Box::new(JsonFormatObservabilityModuleLoader),
    ]
}

/// Helper struct to hold configuration loading results
struct ConfigLoadResult {
    loaders: Vec<Box<dyn ModuleLoader>>,
    config_adapter: Box<dyn ConfigurationAdapter>,
    config_adapter_params: HashMap<String, String>,
}

/// Load configuration adapters from loaders and resolve the appropriate adapter
fn load_config_adapters(
    config_path: Option<String>,
    config_params: Option<String>,
    config_adapter_name: Option<String>,
) -> Result<ConfigLoadResult, Box<dyn std::error::Error>> {
    let mut loaders: Vec<Box<dyn ModuleLoader>> = get_loaders();

    let mut config_registry: HashMap<&'static str, Box<dyn ConfigurationAdapter>> = HashMap::new();
    for loader in &mut loaders {
        loader.register_configuration_adapters(&mut config_registry);
    }

    let mut adapter_name = config_adapter_name;
    let mut adapter_params = config_params
        .map(|s| parse_config_params(&s))
        .unwrap_or_default();

    let path = config_path.unwrap_or("./ferron.conf".to_string());

    // Determine configuration adapter based on file extension if not specified
    if adapter_name.is_none() {
        if let Some(ext) = std::path::Path::new(&path)
            .extension()
            .and_then(|s| s.to_str())
        {
            for (name, adapter) in &config_registry {
                if adapter.file_extension().iter().any(|e| e == &ext) {
                    adapter_name = Some(name.to_string());
                    break;
                }
            }
        }
    }

    if adapter_params.is_empty() {
        adapter_params.insert("file".to_string(), path);
    }

    let adapter_name = adapter_name.ok_or(anyhow::anyhow!(
        "Configuration adapter not specified and could not be determined from file extension"
    ))?;

    let config_adapter = config_registry
        .remove(adapter_name.as_str())
        .ok_or(anyhow::anyhow!("Configuration adapter not found"))?;

    Ok(ConfigLoadResult {
        loaders,
        config_adapter,
        config_adapter_params: adapter_params,
    })
}

fn format_location(
    block_name: Option<&str>,
    span: Option<&ferron_core::config::ServerConfigurationSpan>,
) -> String {
    let mut location = String::new();
    if let Some(name) = block_name {
        location.push_str(&format!("block '{}'", name));
    } else {
        location.push_str("global configuration");
    }
    if let Some(span) = span {
        if let Some(file) = &span.file {
            location.push_str(&format!(" in file '{}'", file));
        }
        location.push_str(&format!(" at line {}", span.line));
        location.push_str(&format!(", column {}", span.column));
    }
    location
}
/// Run global and per-protocol configuration validators
fn run_configuration_validators(
    loaders: &mut [Box<dyn ModuleLoader>],
    config: &ferron_core::config::ServerConfiguration,
    global_validator_registry: &[Box<dyn ferron_core::config::validator::ConfigurationValidator>],
    per_protocol_validator_registry: &HashMap<
        &'static str,
        Box<dyn ferron_core::config::validator::ConfigurationValidator>,
    >,
) -> Result<(), Box<dyn std::error::Error>> {
    // Run global validators
    let mut used_global_directives = HashSet::new();
    for validator in global_validator_registry {
        validator
            .validate_block(&config.global_config, &mut used_global_directives, true)
            .map_err(|e| {
                anyhow::anyhow!(
                    "Invalid configuration ({}): {e}",
                    format_location(None, config.global_config.span.as_ref())
                )
            })?;
    }
    let unused_global_directives: Vec<String> = config
        .global_config
        .directives
        .keys()
        .filter(|d| !used_global_directives.contains(*d))
        .cloned()
        .collect();
    for directive in unused_global_directives {
        log_warn!(
            "Unused directive ({}): {directive}",
            format_location(None, config.global_config.span.as_ref())
        );
    }

    // Run per-protocol validators
    let mut config_blocks_registry = HashMap::new();
    for loader in loaders {
        loader.register_per_protocol_configuration_blocks(config, &mut config_blocks_registry);
    }
    for (protocol, blocks) in &config_blocks_registry {
        if let Some(validator) = per_protocol_validator_registry.get(protocol) {
            for block in blocks {
                let mut used_directives = HashSet::new();
                validator
                    .validate_block(block.1, &mut used_directives, false)
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Invalid configuration ({}): {e}",
                            format_location(
                                Some(&format!("{protocol} {}", block.0)),
                                config.global_config.span.as_ref()
                            )
                        )
                    })?;
                let unused_directives: Vec<String> = block
                    .1
                    .directives
                    .keys()
                    .filter(|d| !used_directives.contains(*d))
                    .cloned()
                    .collect();
                for directive in unused_directives {
                    log_warn!(
                        "Unused directive ({}): {directive}",
                        format_location(
                            Some(&format!("{protocol} {}", block.0)),
                            config.global_config.span.as_ref()
                        )
                    );
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn run(
    config_path: Option<String>,
    config_params: Option<String>,
    config_adapter: Option<String>,
    verbose: bool,
    service: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger
    let log_level = if verbose {
        LogLevel::Debug
    } else {
        LogLevel::Info
    };

    #[cfg(windows)]
    {
        // Check if running as a Windows service
        if service {
            // Initialize service logger (Event Log)
            ferron_core::logging::init_service_logger(service::SERVICE_NAME, log_level)?;
            ferron_core::log_info!(
                "Starting {} in Windows service mode",
                env!("CARGO_PKG_NAME")
            );

            // Run as Windows service (config is passed via service arguments)
            return service::run_service(config_path, config_params, config_adapter, verbose)
                .map_err(|e| e.into());
        }
    }
    #[cfg(not(windows))]
    let _ = service; // silence unused variable warning

    if !ferron_core::logging::is_init() {
        // Initialize stdio logger for console mode
        ferron_core::logging::init_stdio_logger(log_level)?;
    }

    #[cfg(unix)]
    let _ = daemon::setup_signal_handlers();

    let mut loaders: Vec<Box<dyn ModuleLoader>> = get_loaders();

    let mut config_registry = HashMap::new();
    let mut registry_builder = RegistryBuilder::new();
    let mut global_validator_registry = Vec::new();
    let mut per_protocol_validator_registry = HashMap::new();
    for loader in &mut loaders {
        loader.register_per_protocol_configuration_validators(&mut per_protocol_validator_registry);
        loader.register_global_configuration_validators(&mut global_validator_registry);
        loader.register_configuration_adapters(&mut config_registry);
        registry_builder = loader.register_stages(registry_builder);
        registry_builder = loader.register_providers(registry_builder);
    }
    let registry = registry_builder.build();

    let mut config_adapter_name = config_adapter.as_deref();
    let mut config_adapter_params = config_params
        .map(|s| parse_config_params(&s))
        .unwrap_or_default();

    let path = config_path.unwrap_or("./ferron.conf".to_string());

    // Determine configuration adapter based on file extension if not specified
    if config_adapter_name.is_none() {
        if let Some(ext) = std::path::Path::new(&path)
            .extension()
            .and_then(|s| s.to_str())
        {
            for (name, adapter) in &config_registry {
                if adapter.file_extension().iter().any(|e| e == &ext) {
                    config_adapter_name = Some(name);
                    break;
                }
            }
        }
    }

    if config_adapter_params.is_empty() {
        config_adapter_params.insert("file".to_string(), path);
    }

    let config_adapter_name = config_adapter_name.ok_or(anyhow::anyhow!(
        "Configuration adapter not specified and could not be determined from file extension"
    ))?;

    let config_adapter = config_registry
        .get(config_adapter_name)
        .ok_or(anyhow::anyhow!("Configuration adapter not found"))?;

    load_modules(
        loaders,
        registry,
        config_adapter.as_ref(),
        config_adapter_params,
        global_validator_registry,
        per_protocol_validator_registry,
    )?;

    Ok(())
}

fn validate(
    config_path: Option<String>,
    config_params: Option<String>,
    config_adapter: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let ConfigLoadResult {
        mut loaders,
        config_adapter,
        config_adapter_params,
    } = load_config_adapters(config_path, config_params, config_adapter)?;

    let mut global_validator_registry = Vec::new();
    let mut per_protocol_validator_registry = HashMap::new();
    for loader in &mut loaders {
        loader.register_per_protocol_configuration_validators(&mut per_protocol_validator_registry);
        loader.register_global_configuration_validators(&mut global_validator_registry);
    }

    let (config, _) = config_adapter
        .adapt(&config_adapter_params)
        .map_err(|e| anyhow::anyhow!("Failed to load configuration: {e}"))?;

    run_configuration_validators(
        &mut loaders,
        &config,
        &global_validator_registry,
        &per_protocol_validator_registry,
    )?;

    Ok(())
}

fn adapt(
    config_path: Option<String>,
    config_params: Option<String>,
    config_adapter: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let ConfigLoadResult {
        config_adapter,
        config_adapter_params,
        ..
    } = load_config_adapters(config_path, config_params, config_adapter)?;

    let (config, _) = config_adapter
        .adapt(&config_adapter_params)
        .map_err(|e| anyhow::anyhow!("Failed to load configuration: {e}"))?;
    let json = serde_json::to_string_pretty(&config)?;
    println!("{}", json);

    Ok(())
}

fn load_modules(
    mut loaders: Vec<Box<dyn ModuleLoader>>,
    registry: Arc<Registry>,
    config_adapter: &dyn ConfigurationAdapter,
    config_adapter_params: HashMap<String, String>,
    global_validator_registry: Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    per_protocol_validator_registry: HashMap<
        &'static str,
        Box<dyn ferron_core::config::validator::ConfigurationValidator>,
    >,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = None;

    loop {
        let (mut config, mut watcher) = config_adapter
            .adapt(&config_adapter_params)
            .map_err(|e| anyhow::anyhow!("Failed to load configuration: {e}"))?;

        let mut modules = Vec::new();

        // Configuration validation
        run_configuration_validators(
            &mut loaders,
            &config,
            &global_validator_registry,
            &per_protocol_validator_registry,
        )?;

        let mut layered_config = LayeredConfiguration::new();
        layered_config.add_layer(config.global_config.clone());
        let io_uring_enabled = layered_config
            .get_entry("runtime", false)
            .and_then(|d| d.children.as_ref().map(|c| c.get_flag("io_uring")))
            .unwrap_or(true);

        if runtime.is_none() {
            runtime = Some(Runtime::new(io_uring_enabled)?);
        }
        let mut runtime = runtime
            .take()
            .expect("runtime should be initialized at this point");

        for loader in &mut loaders {
            loader.register_modules(registry.clone(), &mut modules, &mut config)?;
        }

        // Start all modules
        for module in modules {
            log_debug!("Starting module: {}", module.name());
            module.start(&mut runtime)?;
        }

        // Run the runtime (check for shutdown/reload signal)
        let shutdown = runtime.block_on(async move {
            let shutdown_token = SHUTDOWN_TOKEN.load();
            let reload_token = RELOAD_TOKEN.load();
            tokio::select! {
                _ = shutdown_token.cancelled() => {
                    Ok(true)
                }
                _ = reload_token.cancelled() => {
                    Ok(false)
                }
                res = watcher.watch() => {
                    res.map(|_| false)
                }
            }
        })?;

        if shutdown {
            log_info!("Shutting down the server...");
            return Ok(());
        } else {
            log_info!("Reloading configuration...");
        }
    }
}
