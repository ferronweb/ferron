//! Global module registry for Ferron
//!
//! Provides centralized registration and orchestration of server modules and HTTP stages.

use std::sync::Arc;

use parking_lot::RwLock;

use ferron_core::http::HttpContext;
use ferron_module_api::FerronModule;
use ferron_runtime::pipeline::{Pipeline, Stage};

/// A stage factory that can create stage instances by name
pub type StageFactory<C> = Arc<dyn Fn() -> Arc<dyn Stage<C>> + Send + Sync>;

/// Registry for HTTP stages that can be pre-loaded and assembled into pipelines
pub struct StageRegistry<C> {
    stages: RwLock<Vec<(String, StageFactory<C>)>>,
}

impl<C> Default for StageRegistry<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> StageRegistry<C> {
    pub fn new() -> Self {
        Self {
            stages: RwLock::new(Vec::new()),
        }
    }

    /// Register a stage factory with a name
    pub fn register<N, F>(&self, name: N, factory: F)
    where
        N: Into<String>,
        F: Fn() -> Arc<dyn Stage<C>> + Send + Sync + 'static,
    {
        self.stages.write().push((name.into(), Arc::new(factory)));
    }

    /// Build a pipeline from a list of stage names
    pub fn build_pipeline(&self, stage_names: &[&str]) -> Pipeline<C> {
        let mut pipeline = Pipeline::new();
        let stages = self.stages.read();

        for name in stage_names {
            for (registered_name, factory) in stages.iter() {
                if registered_name == name {
                    let stage = factory();
                    pipeline = pipeline.add_stage(stage);
                    break;
                }
            }
        }

        pipeline
    }

    /// Build a pipeline with all registered stages in registration order
    pub fn build_all(&self) -> Pipeline<C> {
        let mut pipeline = Pipeline::new();
        let stages = self.stages.read();

        for (_, factory) in stages.iter() {
            let stage = factory();
            pipeline = pipeline.add_stage(stage);
        }

        pipeline
    }

    /// Get the number of registered stages
    pub fn len(&self) -> usize {
        self.stages.read().len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.stages.read().is_empty()
    }
}

/// Global module registry for pre-loading and orchestrating modules
pub struct ModuleRegistry {
    modules: RwLock<Vec<Arc<dyn FerronModule>>>,
    http_stage_registry: StageRegistry<HttpContext>,
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleRegistry {
    /// Create a new empty module registry
    pub fn new() -> Self {
        Self {
            modules: RwLock::new(Vec::new()),
            http_stage_registry: StageRegistry::new(),
        }
    }

    /// Register a module
    pub fn register<M>(&self, module: M)
    where
        M: FerronModule + 'static,
    {
        self.modules.write().push(Arc::new(module));
    }

    /// Register multiple modules at once
    pub fn register_all<M, I>(&self, modules: I)
    where
        M: FerronModule + 'static,
        I: IntoIterator<Item = M>,
    {
        let mut lock = self.modules.write();
        for module in modules {
            lock.push(Arc::new(module));
        }
    }

    /// Get a reference to the HTTP stage registry
    pub fn http_stages(&self) -> &StageRegistry<HttpContext> {
        &self.http_stage_registry
    }

    /// Get a mutable reference to the HTTP stage registry
    pub fn http_stages_mut(&mut self) -> &mut StageRegistry<HttpContext> {
        &mut self.http_stage_registry
    }

    /// Build and start all server modules, returning join handles
    pub fn start_all(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let modules = self.modules.read();
        let mut handles = Vec::new();

        for module in modules.iter() {
            if let Some(server) = module.server() {
                handles.push(tokio::spawn(server.start()));
            }
        }

        handles
    }

    /// Get all registered modules
    pub fn modules(&self) -> Vec<Arc<dyn FerronModule>> {
        self.modules.read().clone()
    }

    /// Get the number of registered modules
    pub fn len(&self) -> usize {
        self.modules.read().len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.modules.read().is_empty()
    }

    /// Orchestrate the full startup sequence:
    /// 1. Build HTTP pipelines using registered stages
    /// 2. Register pipelines with HTTP modules
    /// 3. Start all server modules
    pub fn orchestrate(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let modules = self.modules.read();

        // First pass: register HTTP pipelines for modules that need them
        for module in modules.iter() {
            if let Some(http_module) = module.http() {
                let pipeline = self.http_stage_registry.build_all();
                let _ = http_module.register(pipeline);
            }
        }

        // Second pass: start all server modules
        let mut handles = Vec::new();
        for module in modules.iter() {
            if let Some(server) = module.server() {
                handles.push(tokio::spawn(server.start()));
            }
        }

        handles
    }
}

/// Builder for creating a ModuleRegistry with a fluent API
pub struct ModuleRegistryBuilder {
    registry: ModuleRegistry,
}

impl Default for ModuleRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleRegistryBuilder {
    pub fn new() -> Self {
        Self {
            registry: ModuleRegistry::new(),
        }
    }

    /// Register a module
    pub fn with_module<M>(self, module: M) -> Self
    where
        M: FerronModule + 'static,
    {
        self.registry.register(module);
        self
    }

    /// Register multiple modules
    pub fn with_modules<M, I>(self, modules: I) -> Self
    where
        M: FerronModule + 'static,
        I: IntoIterator<Item = M>,
    {
        self.registry.register_all(modules);
        self
    }

    /// Register an HTTP stage
    pub fn with_http_stage<N, F>(self, name: N, factory: F) -> Self
    where
        N: Into<String>,
        F: Fn() -> Arc<dyn Stage<HttpContext>> + Send + Sync + 'static,
    {
        self.registry.http_stage_registry.register(name, factory);
        self
    }

    /// Build the registry
    pub fn build(self) -> ModuleRegistry {
        self.registry
    }

    /// Build and start all modules
    pub fn start(self) -> Vec<tokio::task::JoinHandle<()>> {
        let registry = self.build();
        registry.orchestrate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ferron_module_api::{HttpModule, Module, ProvidesHttp, ProvidesServer, ServerModule};
    use std::any::Any;

    struct TestModule {
        name: String,
    }

    impl TestModule {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
            }
        }
    }

    impl Module for TestModule {
        fn name(&self) -> &str {
            &self.name
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl ProvidesServer for TestModule {
        fn server(&self) -> Option<&dyn ServerModule> {
            None
        }
    }

    impl ProvidesHttp for TestModule {
        fn http(&self) -> Option<&dyn HttpModule> {
            None
        }
    }

    // FerronModule is automatically implemented via blanket impl

    struct TestStage;

    #[async_trait]
    impl Stage<HttpContext> for TestStage {
        async fn run(&self, _ctx: &mut HttpContext) {}
    }

    #[test]
    fn test_module_registry() {
        let registry = ModuleRegistry::new();
        registry.register(TestModule::new("test"));

        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_stage_registry() {
        let registry = StageRegistry::new();
        registry.register("test", || Arc::new(TestStage));

        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_builder() {
        let registry = ModuleRegistryBuilder::new()
            .with_module(TestModule::new("test1"))
            .with_module(TestModule::new("test2"))
            .with_http_stage("logging", || Arc::new(TestStage))
            .build();

        assert_eq!(registry.len(), 2);
        assert_eq!(registry.http_stages().len(), 1);
    }
}
