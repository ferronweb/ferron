use std::collections::HashMap;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ferron")]
#[command(about = "Ferron web server CLI", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

// TODO: `ferron service` subcommand for Unix-like systems
#[derive(Subcommand)]
pub enum Commands {
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
    /// Runs the web server as a Unix daemon (Unix only)
    #[cfg(unix)]
    Daemon {
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

        /// Path to the PID file
        #[arg(long = "pid-file")]
        pid_file: Option<String>,
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
pub enum WinServiceCommands {
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

pub fn parse_config_params(params_str: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for pair in params_str.split(';') {
        if let Some((key, value)) = pair.split_once('=') {
            params.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    params
}
