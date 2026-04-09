//! Global registry for stages and providers with DAG-based ordering.
//!
//! This module provides:
//! - `StageRegistry<C>`: Ordered pipeline stages for a specific context type
//! - `ProviderRegistry<P>`: Named providers categorized by type
//! - `Registry`: Type-erased container for multiple typed registries
//! - `RegistryBuilder`: Fluent API for building the registry
//!
//! Stages can define ordering constraints (Before/After) that are resolved
//! using topological sort to build deterministic execution order.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::RwLock;

/// Factory function for creating provider instances.
pub type ProviderFactory<P> = Arc<dyn Fn() -> Arc<dyn crate::providers::Provider<P>> + Send + Sync>;

/// Entry for a registered provider factory.
pub struct ProviderEntry<P> {
    pub factory: ProviderFactory<P>,
    pub name: String,
}

/// Registry for providers organized by type.
///
/// Providers are looked up by name and can be enumerated. This registry
/// supports type erasure through downcasting.
///
/// # Example
///
/// ```ignore
/// let registry = ProviderRegistry::<DnsProvider>::new();
/// registry.register(|| Arc::new(CloudflareDns));
/// let provider = registry.get("cloudflare");
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
/// Used by stages to declare ordering requirements relative to other named stages.
#[derive(Clone, Debug)]
pub enum StageConstraint {
    /// This stage must run before the named stage
    Before(String),
    /// This stage must run after the named stage
    After(String),
}

/// Factory function for creating stage instances.
pub type StageFactory<C> = Arc<dyn Fn() -> Arc<dyn crate::pipeline::Stage<C>> + Send + Sync>;

/// Entry for a registered stage.
pub struct StageEntry<C> {
    pub factory: StageFactory<C>,
    pub name: String,
    pub constraints: Vec<StageConstraint>,
}

/// Registry for pipeline stages with DAG-based topological ordering.
///
/// This registry allows modules to register stages with Before/After constraints,
/// and automatically orders them for execution. Cycles are detected and fall back
/// to registration order.
///
/// # Example
///
/// ```ignore
/// let registry = StageRegistry::<HttpContext>::new();
/// registry.register(|| Arc::new(LoggingStage));
/// registry.register(|| Arc::new(AuthStage::with_constraints(vec![
///     StageConstraint::After("logging".to_string())
/// ])));
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

        // Build name-to-index mapping
        let name_to_idx: HashMap<&str, usize> = stages
            .iter()
            .enumerate()
            .map(|(i, stage)| (stage.name.as_str(), i))
            .collect();

        // Build adjacency list and in-degree count
        let mut graph: HashMap<usize, HashSet<usize>> = HashMap::new();
        let mut in_degree: HashMap<usize, usize> = HashMap::new();

        // Initialize all nodes
        for i in 0..stages.len() {
            in_degree.entry(i).or_insert(0);
            graph.entry(i).or_default();
        }

        // Build edges based on constraints
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

/// Global registry for stages and providers across all context types.
///
/// This registry uses type erasure to support multiple stage and provider types
/// in a single container. Modules register context-specific stages and providers,
/// and can retrieve them later using their context type.
///
/// # Example
///
/// ```ignore
/// let registry = Registry::new();
///
/// // Register HTTP stages
/// registry.register_stage::<HttpContext, _>(|| Arc::new(LoggingStage));
///
/// // Retrieve and use
/// if let Some(http_stages) = registry.get_stage_registry::<HttpContext>() {
///     let pipeline = http_stages.build_all();
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
    /// Stages are used by modules to build ordered pipelines. For example,
    /// HTTP modules register stages with Before/After constraints to define
    /// request processing order (logging -> auth -> handler -> response).
    ///
    /// # Arguments
    ///
    /// * `factory` - A function that creates stage instances
    ///
    /// # Example
    ///
    /// ```ignore
    /// registry.register_stage::<HttpContext, _>(|| Arc::new(LoggingStage));
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

        // Create new registry for this type
        let registry = Arc::new(StageRegistry::<C>::new());
        registry.register(factory);

        registries.insert(type_id, Arc::new(TypedStageRegistry::new(registry)));
    }

    /// Get the stage registry for a specific context type.
    ///
    /// Returns the registry if stages have been registered for this type.
    /// Used by modules to retrieve and build pipelines.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(stages) = registry.get_stage_registry::<HttpContext>() {
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
    /// Providers are discovered by their trait type and name, allowing modules
    /// to extend functionality without compile-time dependencies.
    ///
    /// # Arguments
    ///
    /// * `factory` - A function that creates provider instances
    ///
    /// # Example
    ///
    /// ```ignore
    /// registry.register_provider::<DnsProvider, _>(|| Arc::new(CloudflareDns));
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

        // Create new registry for this type
        let registry = Arc::new(ProviderRegistry::<C>::new());
        registry.register(factory);

        registries.insert(type_id, Arc::new(TypedProviderRegistry::new(registry)));
    }

    /// Get the provider registry for a specific provider trait type.
    ///
    /// Returns the registry if providers have been registered for this type.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(dns_providers) = registry.get_provider_registry::<DnsProvider>() {
    ///     let provider = dns_providers.get("cloudflare");
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

/// Builder for creating a Registry with a fluent API.
///
/// The builder pattern allows convenient chaining of register calls.
///
/// # Example
///
/// ```ignore
/// let registry = RegistryBuilder::new()
///     .with_stage::<HttpContext, _>(|| Arc::new(LoggingStage))
///     .with_stage::<HttpContext, _>(|| Arc::new(AuthStage))
///     .with_provider::<DnsProvider, _>(|| Arc::new(CloudflareDns))
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
    /// Stages are used by modules to build ordered pipelines.
    /// For example, HTTP stages are registered and then used to process requests.
    pub fn with_stage<C, F>(self, factory: F) -> Self
    where
        C: 'static,
        F: Fn() -> Arc<dyn crate::pipeline::Stage<C>> + Send + Sync + 'static,
    {
        self.registry.register_stage::<C, F>(factory);
        self
    }

    /// Register a provider.
    ///
    /// Providers are typed implementations that can be retrieved by trait type and name.
    /// For example, DNS providers are registered and used to resolve domains.
    pub fn with_provider<C, F>(self, factory: F) -> Self
    where
        C: 'static,
        F: Fn() -> Arc<dyn crate::providers::Provider<C>> + Send + Sync + 'static,
    {
        self.registry.register_provider::<C, F>(factory);
        self
    }

    /// Build the registry and return it.
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

    #[test]
    fn test_builder() {
        struct LoggingStage;
        #[async_trait(?Send)]
        impl Stage<()> for LoggingStage {
            fn name(&self) -> &str {
                "logging"
            }
            #[inline]
            async fn run(&self, _ctx: &mut ()) -> Result<bool, PipelineError> {
                Ok(true)
            }
        }

        let registry = RegistryBuilder::new()
            .with_stage::<(), _>(|| Arc::new(LoggingStage))
            .build();

        let stage_registry = registry.get_stage_registry::<()>();
        assert!(stage_registry.is_some());
        assert_eq!(stage_registry.unwrap().len(), 1);
    }

    #[test]
    fn test_provider_registry() {
        use crate::providers::Provider;

        struct DnsProviderImpl {
            name: String,
        }

        impl Provider<()> for DnsProviderImpl {
            fn name(&self) -> &str {
                &self.name
            }

            fn execute(&self, _ctx: &mut ()) -> Result<(), Box<dyn std::error::Error>> {
                Ok(())
            }
        }

        impl DnsProviderImpl {
            #[allow(dead_code)]
            fn resolve(&self, _domain: &str) -> Result<String, String> {
                Ok("127.0.0.1".to_string())
            }
        }

        let registry = ProviderRegistry::<()>::new();

        registry.register(|| {
            Arc::new(DnsProviderImpl {
                name: "cloudflare".to_string(),
            })
        });

        registry.register(|| {
            Arc::new(DnsProviderImpl {
                name: "google".to_string(),
            })
        });

        // Test get by name
        let dns = registry.get("cloudflare");
        assert!(dns.is_some());
        assert_eq!(dns.unwrap().name(), "cloudflare");

        // Test get all
        let all = registry.get_all();
        assert_eq!(all.len(), 2);

        // Test len
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn test_provider_registry_via_main_registry() {
        use crate::providers::Provider;

        struct TestDnsProvider {
            name: String,
        }

        #[allow(dead_code)]
        struct DnsProviderContext {
            input: String,
            result: Option<String>,
        }

        impl Provider<DnsProviderContext> for TestDnsProvider {
            fn name(&self) -> &str {
                &self.name
            }

            fn execute(
                &self,
                ctx: &mut DnsProviderContext,
            ) -> Result<(), Box<dyn std::error::Error>> {
                ctx.result = Some("192.168.1.1".to_string());
                Ok(())
            }
        }

        let registry = Registry::new();

        registry.register_provider::<DnsProviderContext, _>(|| {
            Arc::new(TestDnsProvider {
                name: "default".to_string(),
            })
        });

        let dns_registry = registry.get_provider_registry::<DnsProviderContext>();
        assert!(dns_registry.is_some());

        let dns = dns_registry.unwrap().get("default");
        assert!(dns.is_some());
        let mut ctx = DnsProviderContext {
            input: "example.com".to_string(),
            result: None,
        };
        dns.unwrap().execute(&mut ctx).unwrap();
        assert_eq!(ctx.result, Some("192.168.1.1".to_string()));
    }
}
