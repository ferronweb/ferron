use std::{collections::HashMap, error::Error, sync::Arc};

use ferron_common::{dns::DnsProvider, modules::ModuleLoader};

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

pub fn get_dns_provider(
  provider_name: &str,
  challenge_params: &HashMap<String, String>,
) -> Result<Arc<dyn DnsProvider + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(include!(concat!(env!("OUT_DIR"), "/match_dns_providers.rs")))
}
