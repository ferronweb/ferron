use std::sync::Arc;

use ferron_http::{HttpContext, HttpFileContext};

use crate::stages::{CgiInjectStage, CgiStage};

mod config;
mod stages;
mod util;
mod validator;

pub struct CgiModuleLoader;

impl ferron_core::loader::ModuleLoader for CgiModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(validator::CgiConfigurationValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            &'static str,
            Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(validator::CgiConfigurationValidator));
    }

    fn register_stages(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry
            .with_stage::<HttpFileContext, _>(|| Arc::new(CgiStage))
            .with_stage::<HttpContext, _>(|| Arc::new(CgiInjectStage))
    }
}
