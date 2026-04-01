use std::{collections::HashMap, sync::Arc};

use crate::config::adapter::ConfigurationAdapter;

pub trait ModuleLoader {
    #[allow(unused_variables)]
    fn register_per_protocol_configuration_blocks<'a>(
        &mut self,
        config: &'a crate::config::ServerConfiguration,
        registry: &mut HashMap<
            &'static str,
            Vec<(String, &'a crate::config::ServerConfigurationBlock)>,
        >,
    ) {
    }

    #[allow(unused_variables)]
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn crate::config::validator::ConfigurationValidator>>,
    ) {
    }

    #[allow(unused_variables)]
    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut HashMap<
            &'static str,
            Box<dyn crate::config::validator::ConfigurationValidator>,
        >,
    ) {
    }

    #[allow(unused_variables)]
    fn register_configuration_adapters(
        &mut self,
        registry: &mut HashMap<&'static str, Box<dyn ConfigurationAdapter>>,
    ) {
    }

    fn register_stages(
        &mut self,
        registry: crate::registry::RegistryBuilder,
    ) -> crate::registry::RegistryBuilder {
        registry
    }

    #[allow(unused_variables)]
    fn register_providers(
        &mut self,
        registry: crate::registry::RegistryBuilder,
    ) -> crate::registry::RegistryBuilder {
        registry
    }

    #[allow(unused_variables)]
    fn register_modules(
        &mut self,
        registry: &crate::registry::Registry,
        modules: &mut Vec<Arc<dyn crate::Module>>,
        config: &mut crate::config::ServerConfiguration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}
