#[cfg(not(unix))]
compile_error!("This configuration adapter is supported only on Unix and Unix-like systems.");

use std::error::Error;
use std::fs;
use std::path::Path;

use crate::config::{
  adapters::{kdl::KdlConfigurationAdapter, ConfigurationAdapter},
  ServerConfiguration,
};

/// An automatic configuration adapter for use in Docker deployments
pub struct DockerAutoConfigurationAdapter;

impl DockerAutoConfigurationAdapter {
  /// Creates a new configuration adapter
  pub fn new() -> Self {
    Self
  }
}

impl ConfigurationAdapter for DockerAutoConfigurationAdapter {
  fn load_configuration(
    &self,
    _path: &Path,
  ) -> Result<Vec<ServerConfiguration>, Box<dyn Error + Send + Sync>> {
    #[cfg(feature = "config-yaml-legacy")]
    {
      use crate::config::adapters::yaml_legacy::YamlLegacyConfigurationAdapter;
      if fs::exists("/etc/ferron.yaml")? {
        return Ok(
          YamlLegacyConfigurationAdapter::new()
            .load_configuration(Path::new("/etc/ferron.yaml"))?,
        );
      } else if fs::exists("/etc/ferron.yml")? {
        return Ok(
          YamlLegacyConfigurationAdapter::new().load_configuration(Path::new("/etc/ferron.yml"))?,
        );
      }
    }

    if fs::exists("/etc/ferron.kdl")? {
      return Ok(KdlConfigurationAdapter::new().load_configuration(Path::new("/etc/ferron.kdl"))?);
    }

    Err(anyhow::anyhow!("Can't detect the server configuration."))?
  }
}
