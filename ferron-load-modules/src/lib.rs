use ferron_common::modules::ModuleLoader;

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
