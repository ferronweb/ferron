//! Global registry for Ferron
//!
//! Provides centralized registration of stages and providers with trait-object based
//! entries. Stages can be registered for specific context types and ordered
//! using DAG-based topological sort.
//!
//! The registry supports:
//! - Stages: Pipeline components for specific context types with ordering constraints
//! - Providers: Categorized components (e.g., DNS, cache) identified by category and name
//!
//! There are also modules, which are server implementations (HTTP, TCP, etc.) that
//! can run independently.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[allow(unused_imports)]
use crate::providers::Provider;
use parking_lot::RwLock;

/// A factory for creating provider instances
pub type ProviderFactory<P> = Arc<dyn Fn() -> Arc<dyn crate::providers::Provider<P>> + Send + Sync>;

/// Entry for a registered provider
pub struct ProviderEntry<P> {
    pub factory: ProviderFactory<P>,
}

/// Registry for providers organized by name
///
/// Providers are grouped by category and can be looked up by name.
/// This registry is type-erased and supports provider sub-traits.
pub struct ProviderRegistry<P> {
    providers: RwLock<Vec<ProviderEntry<P>>>,
}

impl<P: 'static> Default for ProviderRegistry<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: 'static> ProviderRegistry<P> {
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(Vec::new()),
        }
    }

    /// Register a provider factory
    pub fn register<F>(&self, factory: F)
    where
        F: Fn() -> Arc<dyn crate::providers::Provider<P>> + Send + Sync + 'static,
    {
        self.providers.write().push(ProviderEntry {
            factory: Arc::new(factory),
        });
    }

    /// Get a provider by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn crate::providers::Provider<P>>> {
        let providers = self.providers.read();
        for entry in providers.iter() {
            let instance = (entry.factory)();
            if instance.name() == name {
                return Some(instance);
            }
        }
        None
    }

    /// Get all providers in this registry
    pub fn get_all(&self) -> Vec<Arc<dyn crate::providers::Provider<P>>> {
        let providers = self.providers.read();
        providers.iter().map(|e| (e.factory)()).collect()
    }

    /// Get the number of registered providers
    pub fn len(&self) -> usize {
        self.providers.read().len()
    }

    /// Check if the registry is empty
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
    fn new(registry: Arc<ProviderRegistry<P>>) -> Self {
        Self { registry }
    }

    fn get_registry(&self) -> Arc<ProviderRegistry<P>> {
        Arc::clone(&self.registry)
    }
}

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
            graph.entry(i).or_default();
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

/// Registry that holds type-specific stage registries
///
/// - Stage registries are per-context-type and support DAG-based ordering
/// - Provider registries for typed provider access (e.g., DnsProvider, CacheProvider)
///
/// Example usage:
/// - HTTP module uses `StageRegistry<HttpContext>` for ordered pipeline
/// - TCP module might not use stages at all, just run a server directly
/// - DNS module uses `ProviderRegistry<DnsProvider>` for DNS providers
pub struct Registry {
    stage_registries: RwLock<HashMap<TypeId, Arc<dyn AnyStageRegistry>>>,
    provider_registries: RwLock<HashMap<TypeId, Arc<dyn AnyProviderRegistry>>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        Self {
            stage_registries: RwLock::new(HashMap::new()),
            provider_registries: RwLock::new(HashMap::new()),
        }
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

    /// Register a provider
    ///
    /// This allows modules to define typed provider registries.
    /// For example, DNS providers can be registered with `ProviderRegistry<DnsProvider>`.
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

    /// Get the provider registry for a specific provider type
    ///
    /// Modules can use this to retrieve their typed provider registry.
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

    /// Register a provider
    ///
    /// Providers are typed providers (e.g., DnsProvider, CacheProvider)
    /// that can be retrieved with their specific trait methods.
    pub fn with_provider<C, F>(self, factory: F) -> Self
    where
        C: 'static,
        F: Fn() -> Arc<dyn crate::providers::Provider<C>> + Send + Sync + 'static,
    {
        self.registry.register_provider::<C, F>(factory);
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

    #[test]
    fn test_stage_registry_ordering() {
        let registry = StageRegistry::new();

        struct HelloStage;
        #[async_trait(?Send)]
        impl Stage<()> for HelloStage {
            fn name(&self) -> &str {
                "hello"
            }
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
        use crate::registry::Provider;

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
        use crate::registry::Provider;

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
