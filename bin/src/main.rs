// TODO: replace "println!" and "eprintln!" with custom logging macro usage
// TODO: map SHUTDOWN_TOKEN to SIGINT, and RELOAD_TOKEN to SIGHUP on Unix-like systems

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use ferron_config::BlankConfigurationAdapterModuleLoader;
use ferron_core::config::adapter::ConfigurationAdapter;
use ferron_core::loader::ModuleLoader;
use ferron_core::logging::LogLevel;
use ferron_core::registry::{Registry, RegistryBuilder};
use ferron_core::runtime::Runtime;
use ferron_core::shutdown::{RELOAD_TOKEN, SHUTDOWN_TOKEN};
use ferron_http::BasicHttpModuleLoader;

mod service;

#[derive(Parser)]
#[command(name = "ferron")]
#[command(about = "Ferron web server CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

// TODO: `ferron service` subcommand for Unix-like systems
#[derive(Subcommand)]
enum Commands {
    /// Starts the web server
    Run {
        /// Path to the configuration file
        #[arg(short = 'c', long = "config")]
        config_path: Option<String>,

        /// Configuration parameters in key=value;key2=value2 format
        #[arg(long = "config-params")]
        config_params: Option<String>,

        /// Configuration adapter name
        #[arg(long = "config-adapter")]
        config_adapter: Option<String>,

        /// Enable verbose (debug) logging
        #[arg(long = "verbose", short = 'v')]
        verbose: bool,

        /// Run as a Windows service (Windows only)
        #[cfg(windows)]
        #[arg(long = "service")]
        service: bool,
    },
    /// Validates the web server configuration
    Validate {
        /// Path to the configuration file
        #[arg(short = 'c', long = "config")]
        config_path: Option<String>,

        /// Configuration parameters in key=value;key2=value2 format
        #[arg(long = "config-params")]
        config_params: Option<String>,

        /// Configuration adapter name
        #[arg(long = "config-adapter")]
        config_adapter: Option<String>,
    },
    /// Translates the web server configuration into JSON representation
    Adapt {
        /// Path to the configuration file
        #[arg(short = 'c', long = "config")]
        config_path: Option<String>,

        /// Configuration parameters in key=value;key2=value2 format
        #[arg(long = "config-params")]
        config_params: Option<String>,

        /// Configuration adapter name
        #[arg(long = "config-adapter")]
        config_adapter: Option<String>,
    },
    /// Windows service management (Windows only)
    #[cfg(windows)]
    #[command(name = "winservice")]
    WinService {
        #[command(subcommand)]
        subcommand: WinServiceCommands,
    },
}

#[cfg(windows)]
#[derive(Subcommand)]
enum WinServiceCommands {
    /// Install the Windows service
    Install {
        /// Path to the configuration file
        #[arg(short = 'c', long = "config")]
        config_path: Option<String>,

        /// Configuration parameters in key=value;key2=value2 format
        #[arg(long = "config-params")]
        config_params: Option<String>,

        /// Configuration adapter name
        #[arg(long = "config-adapter")]
        config_adapter: Option<String>,

        /// Enable verbose (debug) logging
        #[arg(long = "verbose", short = 'v')]
        verbose: bool,
    },
    /// Uninstall the Windows service
    Uninstall,
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

// TODO: report errors into the console
fn main() -> Result<(), Box<dyn std::error::Error>> {
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
        Box::new(BasicHttpModuleLoader::default()),
        Box::new(BlankConfigurationAdapterModuleLoader),
    ]
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

    if !ferron_core::logging::is_init() {
        // Initialize stdio logger for console mode
        ferron_core::logging::init_stdio_logger(log_level)?;
    }

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
    }
    let registry = registry_builder.build();

    let mut config_adapter_name = config_adapter.as_deref();
    let mut config_adapter_params = config_params
        .map(|s| parse_config_params(&s))
        .unwrap_or_default();
    if let Some(path) = config_path {
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
    _config_path: Option<String>,
    _config_params: Option<String>,
    _config_adapter: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Validating configuration...");
    // TODO: implement configuration validation
    println!("Configuration validation TODO");
    Ok(())
}

fn adapt(
    config_path: Option<String>,
    config_params: Option<String>,
    config_adapter: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut loaders: Vec<Box<dyn ModuleLoader>> = get_loaders();

    let mut config_registry = HashMap::new();
    let mut global_validator_registry = Vec::new();
    let mut per_protocol_validator_registry = HashMap::new();
    for loader in &mut loaders {
        loader.register_per_protocol_configuration_validators(&mut per_protocol_validator_registry);
        loader.register_global_configuration_validators(&mut global_validator_registry);
        loader.register_configuration_adapters(&mut config_registry);
    }

    let mut config_adapter_name = config_adapter.as_deref();
    let mut config_adapter_params = config_params
        .map(|s| parse_config_params(&s))
        .unwrap_or_default();
    if let Some(path) = config_path {
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

        config_adapter_params.insert("file".to_string(), path);
    }
    let config_adapter_name = config_adapter_name.ok_or(anyhow::anyhow!(
        "Configuration adapter not specified and could not be determined from file extension"
    ))?;

    let config_adapter = config_registry
        .get(config_adapter_name)
        .ok_or(anyhow::anyhow!("Configuration adapter not found"))?;

    let (config, _) = config_adapter.adapt(&config_adapter_params)?;
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
    let mut runtime = Runtime::new()?;

    loop {
        let (mut config, mut watcher) = config_adapter.adapt(&config_adapter_params)?;

        let mut config_blocks_registry = HashMap::new();
        let mut modules = Vec::new();

        // configuration validation
        let mut unused_global_directives = HashSet::new();
        for validator in &global_validator_registry {
            validator.validate_block(&config.global_config, &mut unused_global_directives)?;
        }
        for directive in unused_global_directives {
            // TODO: specify where are the unused directives in the configuration file
            println!("Warning: unused global directive: {}", directive);
        }
        for loader in &mut loaders {
            loader.register_per_protocol_configuration_blocks(&config, &mut config_blocks_registry);
        }
        for (protocol, blocks) in &config_blocks_registry {
            if let Some(validator) = per_protocol_validator_registry.get(protocol) {
                let mut unused_directives = HashSet::new();
                for block in blocks {
                    validator.validate_block(block.1, &mut unused_directives)?;
                }
                for directive in unused_directives {
                    // TODO: specify where are the unused directives in the configuration file
                    println!(
                        "Warning: unused directive in protocol {}: {}",
                        protocol, directive
                    );
                }
            }
        }

        for loader in &mut loaders {
            loader.register_modules(&registry, &mut modules, &mut config);
        }

        // Start all modules
        for module in modules {
            println!("Starting module: {}", module.name());
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
            return Ok(());
        }
    }
}
