pub trait ModuleLoader {
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
        modules: &mut Vec<Box<dyn crate::Module>>,
    ) {
    }
}
