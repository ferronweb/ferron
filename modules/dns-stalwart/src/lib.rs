mod client;
mod providers;

use ferron_core::loader::ModuleLoader;

pub struct StalwartDnsModuleLoader;

impl ModuleLoader for StalwartDnsModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        providers::register_providers(registry)
    }
}
