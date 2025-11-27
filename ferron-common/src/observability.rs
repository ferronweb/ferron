use std::collections::HashSet;
use std::error::Error;
use std::sync::Arc;

use async_channel::Sender;

use crate::config::ServerConfiguration;
use crate::logging::LogMessage;

/// A trait that defines an observability backend loader
pub trait ObservabilityBackendLoader {
  /// Loads an observability backend according to specific configuration
  fn load_observability_backend(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
    secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn ObservabilityBackend + Send + Sync>, Box<dyn Error + Send + Sync>>;

  /// Determines configuration properties required to load an observability backend
  fn get_requirements(&self) -> Vec<&'static str> {
    vec![]
  }

  /// Validates the server configuration
  #[allow(unused_variables)]
  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }
}

/// A trait that defines an observability backend
pub trait ObservabilityBackend {
  /// Obtains the channel for logging
  fn get_log_channel(&self) -> Option<Sender<LogMessage>> {
    None
  }
}

/// Observability backend channels inside configurations
#[derive(Clone)]
pub struct ObservabilityBackendChannels {
  /// Log channels
  pub log_channels: Vec<Sender<LogMessage>>,
}

impl Default for ObservabilityBackendChannels {
  fn default() -> Self {
    Self::new()
  }
}

impl ObservabilityBackendChannels {
  /// Creates an empty instance of `ObservabilityBackendChannels`
  pub fn new() -> Self {
    Self {
      log_channels: Vec::new(),
    }
  }

  /// Adds a log channel to the observability backend channels
  pub fn add_log_channel(&mut self, channel: Sender<LogMessage>) {
    self.log_channels.push(channel);
  }
}
