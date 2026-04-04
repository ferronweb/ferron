//! Admin API module loader implementation.

use std::ops::Deref;
use std::sync::Arc;

use ferron_admin::ADMIN_METRICS;
use ferron_core::loader::ModuleLoader;

use crate::config::AdminConfig;
use crate::server::AdminApiModule;
use crate::validator::AdminConfigurationValidator;

/// Module loader for the admin API.
///
/// Reads the `admin {}` block from global configuration and creates
/// or reloads the `AdminApiModule`.
#[derive(Default)]
pub struct AdminApiModuleLoader {
    /// Cached admin module for hot-reload support.
    /// Only one admin module can exist (single listen port).
    cache: Option<Arc<AdminApiModule>>,
}

impl ModuleLoader for AdminApiModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(AdminConfigurationValidator));
    }

    fn register_modules(
        &mut self,
        registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Initialize admin metrics
        let _ = ADMIN_METRICS.deref();

        let admin_config = AdminConfig::from_global(&config.global_config);

        match (self.cache.take(), admin_config) {
            // Admin was configured before and still configured now → reload
            (Some(cached), Some(new_config)) => {
                cached.reload(&registry, new_config, config.clone())?;
                modules.push(cached.clone());
                self.cache = Some(cached);
            }
            // Admin was configured before but now removed → drop (don't re-add)
            (Some(_cached), None) => {
                self.cache = None;
            }
            // Admin is newly configured → create new module
            (None, Some(new_config)) => {
                let module = Arc::new(AdminApiModule::new(&registry, new_config, config.clone())?);
                modules.push(module.clone());
                self.cache = Some(module);
            }
            // Admin was never configured and still isn't → nothing to do
            (None, None) => {}
        }

        Ok(())
    }
}
