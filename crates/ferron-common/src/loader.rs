use std::{collections::HashMap, sync::Arc};

use crate::config::adapter::ConfigurationAdapter;

pub trait ModuleLoader {
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
    fn register_modules(
        &mut self,
        registry: &crate::registry::Registry,
        modules: &mut Vec<Arc<dyn crate::Module>>,
    ) {
    }
}
