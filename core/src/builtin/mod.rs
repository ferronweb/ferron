use crate::loader::ModuleLoader;

mod validator;

#[derive(Default)]
pub struct BuiltinModuleLoader;

impl ModuleLoader for BuiltinModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn crate::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(validator::BuiltinGlobalConfigurationValidator));
    }
}
