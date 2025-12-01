use std::{collections::HashMap, error::Error, sync::Arc};

use ferron_common::{dns::DnsProvider, modules::ModuleLoader, observability::ObservabilityBackendLoader};

pub const FERRON_BUILD_YAML: &str = include_str!(concat!(env!("OUT_DIR"), "/ferron-build.yaml"));

/// Obtains the module loaders
pub fn obtain_module_loaders() -> Vec<Box<dyn ModuleLoader + Send + Sync>> {
  // Module loaders
  let mut module_loaders: Vec<Box<dyn ModuleLoader + Send + Sync>> = Vec::new();

  // Module loader registration macro
  macro_rules! register_module_loader {
    ($moduleloader:expr) => {
      module_loaders.push(Box::new($moduleloader));
    };
  }

  // Register module loaders
  include!(concat!(env!("OUT_DIR"), "/register_module_loaders.rs"));

  // Return the module loaders vector
  module_loaders
}

/// Obtains the observability backend loaders
pub fn obtain_observability_backend_loaders() -> Vec<Box<dyn ObservabilityBackendLoader + Send + Sync>> {
  // Observability backend loaders
  let mut observability_backend_loaders: Vec<Box<dyn ObservabilityBackendLoader + Send + Sync>> = Vec::new();

  // Observability backend loader registration macro
  macro_rules! register_observability_backend_loader {
    ($observability_backend_loader:expr) => {
      observability_backend_loaders.push(Box::new($observability_backend_loader));
    };
  }

  // Register observability backend loaders
  include!(concat!(env!("OUT_DIR"), "/register_observability_backend_loaders.rs"));

  // Return the observability backend loaders vector
  observability_backend_loaders
}

pub fn get_dns_provider(
  provider_name: &str,
  challenge_params: &HashMap<String, String>,
) -> Result<Arc<dyn DnsProvider + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(include!(concat!(env!("OUT_DIR"), "/match_dns_providers.rs")))
}
