#[cfg(not(unix))]
compile_error!("This configuration adapter is supported only on Unix and Unix-like systems.");

use std::fs;
use std::path::Path;
use std::{collections::HashMap, error::Error};

use ferron_common::config::{
  ServerConfigurationEntries, ServerConfigurationEntry, ServerConfigurationFilters, ServerConfigurationValue,
};

use crate::config::{
  adapters::{kdl::KdlConfigurationAdapter, ConfigurationAdapter},
  ServerConfiguration,
};

/// Internal function to load the configuration from the system
fn load_configuration_inner() -> Result<Vec<ServerConfiguration>, Box<dyn Error + Send + Sync>> {
  #[cfg(feature = "config-yaml-legacy")]
  {
    use crate::config::adapters::yaml_legacy::YamlLegacyConfigurationAdapter;
    if fs::exists("/etc/ferron.yaml")? {
      return YamlLegacyConfigurationAdapter::new().load_configuration(Path::new("/etc/ferron.yaml"));
    } else if fs::exists("/etc/ferron.yml")? {
      return YamlLegacyConfigurationAdapter::new().load_configuration(Path::new("/etc/ferron.yml"));
    }
  }

  if fs::exists("/etc/ferron.kdl")? {
    return KdlConfigurationAdapter::new().load_configuration(Path::new("/etc/ferron.kdl"));
  }

  Err(anyhow::anyhow!("Can't detect the server configuration."))?
}

/// An automatic configuration adapter for use in Docker deployments
pub struct DockerAutoConfigurationAdapter;

impl DockerAutoConfigurationAdapter {
  /// Creates a new configuration adapter
  pub fn new() -> Self {
    Self
  }
}

impl ConfigurationAdapter for DockerAutoConfigurationAdapter {
  fn load_configuration(&self, _path: &Path) -> Result<Vec<ServerConfiguration>, Box<dyn Error + Send + Sync>> {
    let mut configuration = load_configuration_inner()?;

    // Embedded global configuration
    // Equivalent to this KDL configuration:
    //
    // globals {
    //   auto_tls_cache "/var/cache/ferron-acme"
    // }
    let mut embedded_global_configuration_entries = HashMap::new();
    embedded_global_configuration_entries.insert(
      "auto_tls_cache".to_string(),
      ServerConfigurationEntries {
        inner: vec![ServerConfigurationEntry {
          values: vec![ServerConfigurationValue::String("/var/cache/ferron-acme".to_string())],
          props: HashMap::new(),
        }],
      },
    );
    let embedded_global_configuration = ServerConfiguration {
      filters: ServerConfigurationFilters {
        is_host: false,
        hostname: None,
        ip: None,
        port: None,
        condition: None,
        error_handler_status: None,
      },
      entries: embedded_global_configuration_entries,
      modules: vec![],
    };

    // Insert the embedded global configuration at the beginning of the configuration vector
    configuration.insert(0, embedded_global_configuration);

    Ok(configuration)
  }
}
