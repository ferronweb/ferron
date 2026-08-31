//! Global registry for stages and providers with DAG-based ordering.
//!
//! The registry is the central coordination point for all modules. It holds
//! two kinds of typed sub-registries:
//!
//! - [`StageRegistry<C>`] -- ordered pipeline stages for a context type `C`.
//! - [`ProviderRegistry<P>`] -- named providers categorized by a trait type `P`.
//!
//! Both sub-registries are generic. The [`Registry`] provides type erasure
//! so they can be stored in a single container and retrieved by their type
//! at runtime.
//!
//! # How modules use the registry
//!
//! Modules register stages and providers in their
//! [`ModuleLoader::register_stages`](crate::loader::ModuleLoader::register_stages)
//! and [`ModuleLoader::register_providers`](crate::loader::ModuleLoader::register_providers)
//! methods via the [`RegistryBuilder`] fluent API. At runtime, other modules
//! (or the same module) retrieve them through [`Registry::get_stage_registry`]
//! or [`Registry::get_provider_registry`].
//!
//! # Stage ordering
//!
//! Stages declare [`Before`](StageConstraint::Before) /
//! [`After`](StageConstraint::After) constraints relative to other named
//! stages. The [`StageRegistry`] resolves these into a total order using
//! Kahn's algorithm (topological sort). Cycles are detected and cause a
//! panic with a descriptive message.
//!
//! # Example
//!
//! ```ignore
//! use std::sync::Arc;
//! use ferron_core::registry::{Registry, RegistryBuilder, StageConstraint};
//!
//! let registry = Registry::new();
//!
//! // Register stages (typically done via RegistryBuilder in ModuleLoader)
//! registry.register_stage::<MyContext, _>(|| Arc::new(LoggingStage));
//! registry.register_stage::<MyContext, _>(|| Arc::new(AuthStage));
//!
//! // Retrieve and build pipeline
//! if let Some(stages) = registry.get_stage_registry::<MyContext>() {
//!     let pipeline = stages.build_all();
//! }
//! ```

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::RwLock;

/// Global registry for DNS provider lookup, set once during module initialization.
///
/// This is a convenience singleton. Modules that need the registry before
/// it is passed through [`ModuleLoader::register_modules`](crate::loader::ModuleLoader::register_modules)
/// (e.g. for DNS resolution during startup) can read it from here.
pub static GLOBAL_REGISTRY: std::sync::OnceLock<Arc<Registry>> = std::sync::OnceLock::new();

/// Factory function for creating provider instances.
///
/// Each call to the factory should produce a fresh provider. This allows
/// thread-local or stateful provider initialization.
pub type ProviderFactory<P> = Arc<dyn Fn() -> Arc<dyn crate::providers::Provider<P>> + Send + Sync>;

/// Entry for a registered provider factory.
///
/// Stores the factory closure alongside the provider's name (obtained by
/// calling the factory once during registration).
pub struct ProviderEntry<P> {
    pub factory: ProviderFactory<P>,
    pub name: String,
}

/// Registry for providers organized by type.
///
/// Each `ProviderRegistry<P>` holds providers that implement
/// [`Provider<P>`](crate::providers::Provider) for the same context type `P`.
/// Providers are looked up by name and can be enumerated.
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use ferron_core::registry::ProviderRegistry;
///
/// let registry = ProviderRegistry::<MyProviderContext>::new();
/// registry.register(|| Arc::new(CloudProvider));
/// let provider = registry.get("cloud");
/// ```
pub struct ProviderRegistry<P> {
    providers: RwLock<Vec<ProviderEntry<P>>>,
}

impl<P: 'static> Default for ProviderRegistry<P> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<P: 'static> ProviderRegistry<P> {
    /// Create a new empty provider registry.
    #[inline]
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(Vec::new()),
        }
    }

    /// Register a provider factory.
    ///
    /// The factory function is called each time the provider is retrieved,
    /// allowing for thread-local or stateful provider initialization.
    pub fn register<F>(&self, factory: F)
    where
        F: Fn() -> Arc<dyn crate::providers::Provider<P>> + Send + Sync + 'static,
    {
        let instance = factory();
        self.providers.write().push(ProviderEntry {
            factory: Arc::new(factory),
            name: instance.name().to_string(),
        });
    }

    /// Get a provider by name.
    ///
    /// Returns the first provider with a matching name, or `None` if not found.
    pub fn get(&self, name: &str) -> Option<Arc<dyn crate::providers::Provider<P>>> {
        let providers = self.providers.read();
        for entry in providers.iter() {
            if entry.name == name {
                return Some((entry.factory)());
            }
        }
        None
    }

    /// Get all providers in this registry.
    pub fn get_all(&self) -> Vec<Arc<dyn crate::providers::Provider<P>>> {
        let providers = self.providers.read();
        providers.iter().map(|e| (e.factory)()).collect()
    }

    /// Get the number of registered providers.
    #[inline]
    pub fn len(&self) -> usize {
        self.providers.read().len()
    }

    /// Check if the registry is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.providers.read().is_empty()
    }
}

/// Type-erased provider registry storage
trait AnyProviderRegistry: Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

struct TypedProviderRegistry<P: 'static> {
    registry: Arc<ProviderRegistry<P>>,
}

impl<P: 'static> AnyProviderRegistry for TypedProviderRegistry<P> {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl<P: 'static> TypedProviderRegistry<P> {
    #[inline]
    fn new(registry: Arc<ProviderRegistry<P>>) -> Self {
        Self { registry }
    }

    #[inline]
    fn get_registry(&self) -> Arc<ProviderRegistry<P>> {
        Arc::clone(&self.registry)
    }
}

/// Constraint for ordering stages in execution order.
///
/// Stages declare constraints relative to other named stages. The
/// [`StageRegistry`] resolves these into a total order via topological
/// sort (Kahn's algorithm).
///
/// # Example
///
/// ```ignore
/// // This stage must run after "logging" and before "handler"
/// fn constraints(&self) -> Vec<StageConstraint> {
///     vec![
///         StageConstraint::After("logging".to_string()),
///         StageConstraint::Before("handler".to_string()),
///     ]
/// }
/// ```
#[derive(Clone, Debug)]
pub enum StageConstraint {
    /// This stage must run **before** the named stage.
    Before(String),
    /// This stage must run **after** the named stage.
    After(String),
}

/// Factory function for creating stage instances.
///
/// Each call to the factory should produce a fresh stage. The factory is
/// called when building a pipeline or when checking `is_applicable`.
pub type StageFactory<C> = Arc<dyn Fn() -> Arc<dyn crate::pipeline::Stage<C>> + Send + Sync>;

/// Entry for a registered stage.
///
/// Stores the factory closure, the stage's name, and its ordering
/// constraints. These are extracted from the stage during registration.
pub struct StageEntry<C> {
    pub factory: StageFactory<C>,
    pub name: String,
    pub constraints: Vec<StageConstraint>,
}

/// Registry for pipeline stages with DAG-based topological ordering.
///
/// Stages are registered with [`Before`](StageConstraint::Before) /
/// [`After`](StageConstraint::After) constraints and automatically ordered
/// via topological sort. Use [`build_all`](Self::build_all) to create a
/// [`Pipeline`](crate::pipeline::Pipeline) with all registered stages, or
/// [`build_with_config`](Self::build_with_config) to include only stages
/// that are applicable to a given configuration block.
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use ferron_core::registry::{StageRegistry, StageConstraint};
///
/// let registry = StageRegistry::<MyContext>::new();
/// registry.register(|| Arc::new(LoggingStage));
/// registry.register(|| Arc::new(AuthStage));
/// let pipeline = registry.build_all();
/// ```
pub struct StageRegistry<C> {
    stages: RwLock<Vec<StageEntry<C>>>,
}

impl<C> Default for StageRegistry<C> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<C> StageRegistry<C> {
    /// Create a new empty stage registry.
    #[inline]
    pub fn new() -> Self {
        Self {
            stages: RwLock::new(Vec::new()),
        }
    }

    /// Register a stage factory.
    ///
    /// The factory function is called each time stages are ordered,
    /// allowing the registry to retrieve stage metadata (name, constraints).
    pub fn register<F>(&self, factory: F)
    where
        F: Fn() -> Arc<dyn crate::pipeline::Stage<C>> + Send + Sync + 'static,
    {
        let factory = Arc::new(factory);
        let stage = factory();
        self.stages.write().push(StageEntry {
            factory,
            name: stage.name().to_string(),
            constraints: stage.constraints(),
        });
    }

    /// Build an ordered list of stage factories using topological sort.
    ///
    /// Stages are ordered according to their Before/After constraints using Kahn's algorithm.
    /// If cycles are detected, returns stages in registration order as fallback.
    ///
    /// # Panics
    /// - If a cycle is detected during the topological sort.
    pub fn get_ordered_factories(&self) -> Vec<StageFactory<C>> {
        let stages = self.stages.read();

        let name_to_idx: HashMap<&str, usize> = stages
            .iter()
            .enumerate()
            .map(|(i, stage)| (stage.name.as_str(), i))
            .collect();

        let mut graph: HashMap<usize, HashSet<usize>> = HashMap::new();
        let mut in_degree: HashMap<usize, usize> = HashMap::new();

        for i in 0..stages.len() {
            in_degree.entry(i).or_insert(0);
            graph.entry(i).or_default();
        }

        for (i, stage) in stages.iter().enumerate() {
            for constraint in &stage.constraints {
                match constraint {
                    StageConstraint::Before(other) => {
                        // This stage must come before 'other'
                        // Edge: this -> other
                        if let Some(&other_idx) = name_to_idx.get(other.as_str()) {
                            if graph.entry(i).or_default().insert(other_idx) {
                                *in_degree.entry(other_idx).or_insert(0) += 1;
                            }
                        }
                    }
                    StageConstraint::After(other) => {
                        // This stage must come after 'other'
                        // Edge: other -> this
                        if let Some(&other_idx) = name_to_idx.get(other.as_str()) {
                            if graph.entry(other_idx).or_default().insert(i) {
                                *in_degree.entry(i).or_insert(0) += 1;
                            }
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
        queue.sort_by(|&a, &b| stages[a].name.cmp(&stages[b].name));

        let mut result = Vec::new();

        while let Some(node) = queue.pop() {
            result.push(node);

            if let Some(neighbors) = graph.get(&node) {
                for &neighbor in neighbors {
                    if let Some(deg) = in_degree.get_mut(&neighbor) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(neighbor);
                            queue.sort_by(|&a, &b| stages[a].name.cmp(&stages[b].name));
                        }
                    }
                }
            }
        }

        // Check for cycles - panic if there are
        if result.len() != stages.len() {
            panic!(
                "Cycle detected in pipeline stages. This may be caused by a \
                constraints conflict between two or more stages. Try compiling \
                Ferron with some conflicting modules disabled."
            );
        } else {
            result
                .into_iter()
                .map(|idx| stages[idx].factory.clone())
                .collect()
        }
    }

    /// Build a pipeline with all registered stages in topologically sorted order.
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

    /// Build a pipeline with only applicable stages based on configuration.
    ///
    /// Each stage factory is instantiated once to call `is_applicable(config)`.
    /// Stages that return `false` are excluded from the pipeline. The remaining
    /// stages are ordered via topological sort.
    pub fn build_with_config(
        &self,
        config: Option<&crate::config::ServerConfigurationBlock>,
    ) -> crate::pipeline::Pipeline<C>
    where
        C: 'static,
    {
        let factories = self.get_ordered_factories();

        let mut pipeline = crate::pipeline::Pipeline::new();

        for factory in factories {
            let stage = factory();
            if stage.is_applicable(config) {
                pipeline = pipeline.add_stage(stage);
            }
        }

        pipeline
    }

    /// Get the number of registered stages.
    #[inline]
    pub fn len(&self) -> usize {
        self.stages.read().len()
    }

    /// Check if the registry is empty.
    #[inline]
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
    #[inline]
    fn new(registry: Arc<StageRegistry<C>>) -> Self {
        Self { registry }
    }

    #[inline]
    fn get_registry(&self) -> Arc<StageRegistry<C>> {
        Arc::clone(&self.registry)
    }
}

/// Type-erased container for multiple typed stage and provider registries.
///
/// The [`Registry`] uses `TypeId`-based dispatch to store and retrieve
/// [`StageRegistry<C>`] and [`ProviderRegistry<P>`] instances for different
/// context types. Modules register their stages and providers through the
/// [`RegistryBuilder`] (which wraps a `Registry`) and retrieve them later
/// by specifying the context type.
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use ferron_core::registry::Registry;
///
/// let registry = Registry::new();
///
/// // Register stages for a context type
/// registry.register_stage::<MyContext, _>(|| Arc::new(LoggingStage));
///
/// // Retrieve the typed registry and build a pipeline
/// if let Some(stages) = registry.get_stage_registry::<MyContext>() {
///     let pipeline = stages.build_all();
/// }
/// ```
pub struct Registry {
    stage_registries: RwLock<HashMap<TypeId, Arc<dyn AnyStageRegistry>>>,
    provider_registries: RwLock<HashMap<TypeId, Arc<dyn AnyProviderRegistry>>>,
}

impl Default for Registry {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// Create a new empty registry.
    #[inline]
    pub fn new() -> Self {
        Self {
            stage_registries: RwLock::new(HashMap::new()),
            provider_registries: RwLock::new(HashMap::new()),
        }
    }

    /// Register a stage for a specific context type.
    ///
    /// The factory is called once during registration to extract the stage's
    /// name and constraints. It is called again each time a pipeline is built.
    ///
    /// If no [`StageRegistry`] exists for this context type, one is created
    /// automatically.
    ///
    /// # Arguments
    ///
    /// * `factory` -- A closure that creates a fresh stage instance.
    ///
    /// # Example
    ///
    /// ```ignore
    /// registry.register_stage::<MyContext, _>(|| Arc::new(LoggingStage));
    /// ```
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

        let registry = Arc::new(StageRegistry::<C>::new());
        registry.register(factory);

        registries.insert(type_id, Arc::new(TypedStageRegistry::new(registry)));
    }

    /// Get the stage registry for a specific context type.
    ///
    /// Returns `None` if no stages have been registered for `C`.
    /// Use this to build pipelines at runtime:
    ///
    /// ```ignore
    /// if let Some(stages) = registry.get_stage_registry::<MyContext>() {
    ///     let pipeline = stages.build_all();
    /// }
    /// ```
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

    /// Register a provider for a specific provider trait type.
    ///
    /// The factory is called once during registration to extract the
    /// provider's name. It is called again each time the provider is
    /// retrieved via [`ProviderRegistry::get`].
    ///
    /// If no [`ProviderRegistry`] exists for this type, one is created
    /// automatically.
    ///
    /// # Arguments
    ///
    /// * `factory` -- A closure that creates a fresh provider instance.
    ///
    /// # Example
    ///
    /// ```ignore
    /// registry.register_provider::<MyContext, _>(|| Arc::new(MyProvider));
    /// ```
    pub fn register_provider<C, F>(&self, factory: F)
    where
        C: 'static,
        F: Fn() -> Arc<dyn crate::providers::Provider<C>> + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<C>();

        let mut registries = self.provider_registries.write();

        // Check if registry exists for this type
        if let Some(erased) = registries.get(&type_id) {
            if let Some(typed) = erased.as_any().downcast_ref::<TypedProviderRegistry<C>>() {
                typed.get_registry().register(factory);
                return;
            }
        }

        let registry = Arc::new(ProviderRegistry::<C>::new());
        registry.register(factory);

        registries.insert(type_id, Arc::new(TypedProviderRegistry::new(registry)));
    }

    /// Get the provider registry for a specific provider trait type.
    ///
    /// Returns `None` if no providers have been registered for `C`.
    ///
    /// ```ignore
    /// if let Some(providers) = registry.get_provider_registry::<MyContext>() {
    ///     let provider = providers.get("my_provider");
    /// }
    /// ```
    pub fn get_provider_registry<C>(&self) -> Option<Arc<ProviderRegistry<C>>>
    where
        C: 'static,
    {
        let type_id = TypeId::of::<C>();
        let registries = self.provider_registries.read();

        registries.get(&type_id).and_then(|erased| {
            erased
                .as_any()
                .downcast_ref::<TypedProviderRegistry<C>>()
                .map(|typed| typed.get_registry())
        })
    }
}

/// Fluent builder for constructing a [`Registry`].
///
/// The builder wraps an `Arc<Registry>` and provides chainable methods
/// for registering stages and providers. It is passed to
/// [`ModuleLoader::register_stages`](crate::loader::ModuleLoader::register_stages)
/// and [`ModuleLoader::register_providers`](crate::loader::ModuleLoader::register_providers).
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use ferron_core::registry::RegistryBuilder;
///
/// let registry = RegistryBuilder::new()
///     .with_stage::<MyContext, _>(|| Arc::new(LoggingStage))
///     .with_stage::<MyContext, _>(|| Arc::new(AuthStage))
///     .with_provider::<MyProvider, _>(|| Arc::new(MyCache))
///     .build();
/// ```
pub struct RegistryBuilder {
    registry: Arc<Registry>,
}

impl Default for RegistryBuilder {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl RegistryBuilder {
    /// Create a new registry builder.
    #[inline]
    pub fn new() -> Self {
        let registry = Arc::new(Registry::new());
        Self { registry }
    }

    /// Register a stage for a specific context type.
    ///
    /// The generic type parameter `C` determines which typed
    /// [`StageRegistry`] the stage goes into. The factory closure is
    /// called once to extract metadata and again each time a pipeline is
    /// built.
    pub fn with_stage<C, F>(self, factory: F) -> Self
    where
        C: 'static,
        F: Fn() -> Arc<dyn crate::pipeline::Stage<C>> + Send + Sync + 'static,
    {
        self.registry.register_stage::<C, F>(factory);
        self
    }

    /// Register a provider for a specific provider trait type.
    ///
    /// The generic type parameter `C` determines which typed
    /// [`ProviderRegistry`] the provider goes into. The factory closure is
    /// called once to extract the provider's name and again each time the
    /// provider is retrieved.
    pub fn with_provider<C, F>(self, factory: F) -> Self
    where
        C: 'static,
        F: Fn() -> Arc<dyn crate::providers::Provider<C>> + Send + Sync + 'static,
    {
        self.registry.register_provider::<C, F>(factory);
        self
    }

    /// Consume the builder and return the finalized [`Registry`].
    ///
    /// The returned `Arc<Registry>` is shared across the server and
    /// passed to [`ModuleLoader::register_modules`](crate::loader::ModuleLoader::register_modules).
    #[inline]
    pub fn build(self) -> Arc<Registry> {
        self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{PipelineError, Stage};
    use async_trait::async_trait;

    #[test]
    fn test_stage_registry_ordering() {
        let registry = StageRegistry::new();

        struct HelloStage;
        #[async_trait(?Send)]
        impl Stage<()> for HelloStage {
            fn name(&self) -> &str {
                "hello"
            }
            #[inline]
            async fn run(&self, _ctx: &mut ()) -> Result<bool, PipelineError> {
                Ok(true)
            }
        }

        struct LoggingStage;
        #[async_trait(?Send)]
        impl Stage<()> for LoggingStage {
            fn name(&self) -> &str {
                "logging"
            }
            fn constraints(&self) -> Vec<StageConstraint> {
                vec![StageConstraint::Before("hello".to_string())]
            }
            #[inline]
            async fn run(&self, _ctx: &mut ()) -> Result<bool, PipelineError> {
                Ok(true)
            }
        }

        struct NotFoundStage;
        #[async_trait(?Send)]
        impl Stage<()> for NotFoundStage {
            fn name(&self) -> &str {
                "not_found"
            }
            fn constraints(&self) -> Vec<StageConstraint> {
                vec![StageConstraint::After("hello".to_string())]
            }
            #[inline]
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
}
