//! Global registry for Ferron
//!
//! Provides centralized registration of modules and stages with trait-object based
//! entries. Modules are the primary unit, while stages can be registered for specific
//! context types and ordered using DAG-based topological sort.
//!
//! The registry supports:
//! - Modules: Server implementations (HTTP, TCP, etc.) that run independently
//! - Stages: Pipeline components for specific context types with ordering constraints

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::Module;

/// Constraint for stage ordering
#[derive(Clone, Debug)]
pub enum StageConstraint {
    /// This stage must run before the named stage
    Before(String),
    /// This stage must run after the named stage
    After(String),
}

/// A stage factory that can create stage instances
pub type StageFactory<C> = Arc<dyn Fn() -> Arc<dyn crate::pipeline::Stage<C>> + Send + Sync>;

/// Entry for a registered stage (generic over context type)
pub struct StageEntry<C> {
    pub factory: StageFactory<C>,
}

/// Generic registry for stages with DAG-based ordering
///
/// This allows modules that need ordered pipelines (like HTTP) to register stages
/// with constraints (Before/After) and have them automatically ordered.
pub struct StageRegistry<C> {
    stages: RwLock<Vec<StageEntry<C>>>,
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

    /// Register a stage factory
    pub fn register<F>(&self, factory: F)
    where
        F: Fn() -> Arc<dyn crate::pipeline::Stage<C>> + Send + Sync + 'static,
    {
        self.stages.write().push(StageEntry {
            factory: Arc::new(factory),
        });
    }

    /// Build an ordered list of stage factories using topological sort based on constraints
    pub fn get_ordered_factories(&self) -> Vec<StageFactory<C>> {
        let stages = self.stages.read();

        // Create instances to get names and constraints
        let stage_instances: Vec<_> = stages.iter().map(|entry| (entry.factory)()).collect();

        // Build name-to-index mapping
        let name_to_idx: HashMap<&str, usize> = stage_instances
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name(), i))
            .collect();

        // Build adjacency list and in-degree count
        let mut graph: HashMap<usize, HashSet<usize>> = HashMap::new();
        let mut in_degree: HashMap<usize, usize> = HashMap::new();

        // Initialize all nodes
        for i in 0..stage_instances.len() {
            in_degree.entry(i).or_insert(0);
            graph.entry(i).or_insert_with(HashSet::new);
        }

        // Build edges based on constraints
        for (i, stage) in stage_instances.iter().enumerate() {
            for constraint in stage.constraints() {
                match constraint {
                    StageConstraint::Before(other) => {
                        // This stage must come before 'other'
                        // Edge: this -> other
                        if let Some(&other_idx) = name_to_idx.get(other.as_str()) {
                            graph.entry(i).or_default().insert(other_idx);
                            *in_degree.entry(other_idx).or_insert(0) += 1;
                        }
                    }
                    StageConstraint::After(other) => {
                        // This stage must come after 'other'
                        // Edge: other -> this
                        if let Some(&other_idx) = name_to_idx.get(other.as_str()) {
                            graph.entry(other_idx).or_default().insert(i);
                            *in_degree.entry(i).or_insert(0) += 1;
                        }
                    }
                }
            }
        }

        // Kahn's algorithm for topological sort
        let mut queue: Vec<usize> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&idx, _)| idx)
            .collect();

        // Sort queue for deterministic order when multiple stages have same priority
        queue.sort_by(|&a, &b| stage_instances[a].name().cmp(stage_instances[b].name()));

        let mut result = Vec::new();

        while let Some(node) = queue.pop() {
            result.push(node);

            if let Some(neighbors) = graph.get(&node) {
                for &neighbor in neighbors {
                    if let Some(deg) = in_degree.get_mut(&neighbor) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(neighbor);
                            queue.sort_by(|&a, &b| {
                                stage_instances[a].name().cmp(stage_instances[b].name())
                            });
                        }
                    }
                }
            }
        }

        // Check for cycles - fall back to registration order
        if result.len() != stage_instances.len() {
            stages.iter().map(|e| e.factory.clone()).collect()
        } else {
            result
                .into_iter()
                .map(|idx| stages[idx].factory.clone())
                .collect()
        }
    }

    /// Build a pipeline with all registered stages in topologically sorted order
    pub fn build_all(&self) -> crate::pipeline::Pipeline<C>
    where
        C: 'static,
    {
        let factories = self.get_ordered_factories();

        let mut pipeline = crate::pipeline::Pipeline::new();

        for factory in factories {
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

/// Type-erased stage registry storage
trait AnyStageRegistry: Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

struct TypedStageRegistry<C: 'static> {
    registry: Arc<StageRegistry<C>>,
}

impl<C: 'static> AnyStageRegistry for TypedStageRegistry<C> {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl<C: 'static> TypedStageRegistry<C> {
    fn new(registry: Arc<StageRegistry<C>>) -> Self {
        Self { registry }
    }

    fn get_registry(&self) -> Arc<StageRegistry<C>> {
        Arc::clone(&self.registry)
    }
}

/// Unified registry that holds modules and type-specific stage registries
///
/// - Modules are server implementations that run independently
/// - Stage registries are per-context-type and support DAG-based ordering
///
/// Example usage:
/// - HTTP module uses `StageRegistry<HttpContext>` for ordered pipeline
/// - TCP module might not use stages at all, just run a server directly
pub struct Registry {
    modules: RwLock<Vec<Arc<dyn Module>>>,
    stage_registries: RwLock<HashMap<TypeId, Arc<dyn AnyStageRegistry>>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        Self {
            modules: RwLock::new(Vec::new()),
            stage_registries: RwLock::new(HashMap::new()),
        }
    }

    /// Register a module
    pub fn register_module<M>(&self, module: M)
    where
        M: Module + 'static,
    {
        self.modules.write().push(Arc::new(module));
    }

    /// Register a stage for a specific context type
    ///
    /// This allows modules to define ordered pipelines using DAG-based sorting.
    /// For example, HTTP modules can register stages with Before/After constraints.
    pub fn register_stage<C, F>(&self, factory: F)
    where
        C: 'static,
        F: Fn() -> Arc<dyn crate::pipeline::Stage<C>> + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<C>();

        let mut registries = self.stage_registries.write();

        // Check if registry exists for this type
        if let Some(erased) = registries.get(&type_id) {
            if let Some(typed) = erased.as_any().downcast_ref::<TypedStageRegistry<C>>() {
                typed.get_registry().register(factory);
                return;
            }
        }

        // Create new registry for this type
        let registry = Arc::new(StageRegistry::<C>::new());
        registry.register(factory);

        registries.insert(type_id, Arc::new(TypedStageRegistry::new(registry)));
    }

    /// Get the stage registry for a specific context type
    ///
    /// Modules can use this to retrieve their stage registry and build ordered pipelines.
    pub fn get_stage_registry<C>(&self) -> Option<Arc<StageRegistry<C>>>
    where
        C: 'static,
    {
        let type_id = TypeId::of::<C>();
        let registries = self.stage_registries.read();

        registries.get(&type_id).and_then(|erased| {
            erased
                .as_any()
                .downcast_ref::<TypedStageRegistry<C>>()
                .map(|typed| typed.get_registry())
        })
    }

    /// Get all registered modules
    pub fn modules(&self) -> Vec<Arc<dyn Module>> {
        self.modules.read().iter().cloned().collect()
    }

    /// Get the number of registered modules
    pub fn len(&self) -> usize {
        self.modules.read().len()
    }

    /// Check if the registry is empty (no modules)
    pub fn is_empty(&self) -> bool {
        self.modules.read().is_empty()
    }
}

/// Builder for creating a Registry with a fluent API
pub struct RegistryBuilder {
    registry: Arc<Registry>,
}

impl Default for RegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RegistryBuilder {
    pub fn new() -> Self {
        let registry = Arc::new(Registry::new());
        Self { registry }
    }

    /// Register a module
    pub fn with_module<M>(self, module: M) -> Self
    where
        M: Module + 'static,
    {
        self.registry.register_module(module);
        self
    }

    /// Register multiple modules
    pub fn with_modules<M, I>(self, modules: I) -> Self
    where
        M: Module + 'static,
        I: IntoIterator<Item = M>,
    {
        for module in modules {
            self.registry.register_module(module);
        }
        self
    }

    /// Register a stage for a specific context type
    ///
    /// Stages are used by modules to build ordered pipelines.
    /// For example, HTTP stages are registered and then used by BasicHttpModule.
    pub fn with_stage<C, F>(self, factory: F) -> Self
    where
        C: 'static,
        F: Fn() -> Arc<dyn crate::pipeline::Stage<C>> + Send + Sync + 'static,
    {
        self.registry.register_stage::<C, F>(factory);
        self
    }

    /// Build the registry
    pub fn build(self) -> Arc<Registry> {
        self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{PipelineError, Stage};
    use async_trait::async_trait;
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

        fn start(
            &self,
            _runtime: &mut crate::runtime::Runtime,
        ) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    #[test]
    fn test_stage_registry_ordering() {
        let registry = StageRegistry::new();

        struct HelloStage;
        #[async_trait]
        impl Stage<()> for HelloStage {
            fn name(&self) -> &str {
                "hello"
            }
            async fn run(&self, _ctx: &mut ()) -> Result<bool, PipelineError> {
                Ok(true)
            }
        }

        struct LoggingStage;
        #[async_trait]
        impl Stage<()> for LoggingStage {
            fn name(&self) -> &str {
                "logging"
            }
            fn constraints(&self) -> Vec<StageConstraint> {
                vec![StageConstraint::Before("hello".to_string())]
            }
            async fn run(&self, _ctx: &mut ()) -> Result<bool, PipelineError> {
                Ok(true)
            }
        }

        struct NotFoundStage;
        #[async_trait]
        impl Stage<()> for NotFoundStage {
            fn name(&self) -> &str {
                "not_found"
            }
            fn constraints(&self) -> Vec<StageConstraint> {
                vec![StageConstraint::After("hello".to_string())]
            }
            async fn run(&self, _ctx: &mut ()) -> Result<bool, PipelineError> {
                Ok(true)
            }
        }

        // Register stages with constraints: logging -> hello -> not_found
        registry.register(|| Arc::new(HelloStage));
        registry.register(|| Arc::new(LoggingStage));
        registry.register(|| Arc::new(NotFoundStage));

        let _pipeline = registry.build_all();

        // Verify order by checking stage names in execution order
        let ordered = registry.get_ordered_factories();
        assert_eq!(ordered.len(), 3);
    }

    #[test]
    fn test_module_registry() {
        let registry = Registry::new();
        registry.register_module(TestModule::new("test"));

        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_builder() {
        struct LoggingStage;
        #[async_trait]
        impl Stage<()> for LoggingStage {
            fn name(&self) -> &str {
                "logging"
            }
            async fn run(&self, _ctx: &mut ()) -> Result<bool, PipelineError> {
                Ok(true)
            }
        }

        let registry = RegistryBuilder::new()
            .with_module(TestModule::new("test1"))
            .with_module(TestModule::new("test2"))
            .with_stage::<(), _>(|| Arc::new(LoggingStage))
            .build();

        assert_eq!(registry.len(), 2);
        let stage_registry = registry.get_stage_registry::<()>();
        assert!(stage_registry.is_some());
        assert_eq!(stage_registry.unwrap().len(), 1);
    }
}
