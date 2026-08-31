//! Pipeline execution framework with ordered stages and inverse operations.
//!
//! A [`Pipeline`] is an ordered sequence of [`Stage`] implementations that
//! execute sequentially. After all stages complete, their
//! [`run_inverse`](Stage::run_inverse) methods are called in reverse order
//! for cleanup.
//!
//! Stages are typically built by [`StageRegistry::build_all`](crate::registry::StageRegistry::build_all)
//! or [`StageRegistry::build_with_config`](crate::registry::StageRegistry::build_with_config),
//! which resolves ordering constraints into a deterministic execution order.
//!
//! # Stage lifecycle
//!
//! 1. `run` is called for each stage in order.
//!    - `Ok(true)` -- continue to the next stage.
//!    - `Ok(false)` -- stop the pipeline gracefully (no error).
//!    - `Err(PipelineError)` -- stop the pipeline with an error.
//! 2. After the forward pass, `run_inverse` is called for every stage that
//!    successfully ran, in reverse order.
//! 3. If any `run_inverse` returns `Err`, the pipeline stops immediately.

use async_trait::async_trait;
use std::sync::Arc;

use crate::config::ServerConfigurationBlock;
use crate::registry::StageConstraint;

/// Error type for pipeline execution failures.
///
/// A [`PipelineError::Terminated`] indicates that a stage requested early
/// but graceful pipeline termination. A [`PipelineError::Custom`] carries
/// a human-readable message from the failing stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    /// Stage requested early pipeline termination
    Terminated,
    /// Custom error from a stage
    Custom(String),
}

impl PipelineError {
    /// Create a custom pipeline error with the given message.
    #[inline]
    pub fn custom(msg: impl Into<String>) -> Self {
        PipelineError::Custom(msg.into())
    }
}

impl std::fmt::Display for PipelineError {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineError::Terminated => write!(f, "pipeline terminated by stage"),
            PipelineError::Custom(msg) => write!(f, "pipeline error: {}", msg),
        }
    }
}

impl std::error::Error for PipelineError {}

/// A processing step in the execution pipeline.
///
/// Stages are the core abstraction for request/response processing in Ferron.
/// Each stage receives a mutable reference to a context `C` and decides
/// whether to continue, stop, or error.
///
/// # Lifecycle
///
/// 1. [`run`](Self::run) is called during the forward pass.
/// 2. [`run_inverse`](Self::run_inverse) is called during the reverse pass
///    (cleanup), only for stages that successfully executed.
///
/// # Ordering
///
/// Stages declare [`Before`](StageConstraint::Before) /
/// [`After`](StageConstraint::After) constraints via [`constraints`](Self::constraints).
/// The [`StageRegistry`](crate::registry::StageRegistry) resolves these
/// into a total order using topological sort.
///
/// # Example
///
/// ```ignore
/// use async_trait::async_trait;
/// use ferron_core::pipeline::{Stage, PipelineError};
/// use ferron_core::registry::StageConstraint;
///
/// struct AuthStage;
///
/// #[async_trait(?Send)]
/// impl Stage<MyContext> for AuthStage {
///     fn name(&self) -> &str { "auth" }
///
///     fn constraints(&self) -> Vec<StageConstraint> {
///         vec![StageConstraint::After("logging".to_string())]
///     }
///
///     async fn run(&self, ctx: &mut MyContext) -> Result<bool, PipelineError> {
///         // ... authenticate request ...
///         Ok(true) // continue to next stage
///     }
///
///     async fn run_inverse(&self, ctx: &mut MyContext) -> Result<(), PipelineError> {
///         // ... cleanup auth state ...
///         Ok(())
///     }
/// }
/// ```
#[async_trait(?Send)]
pub trait Stage<C>: Send + Sync {
    /// Returns the unique name of this stage.
    ///
    /// The name is used for [`StageConstraint::Before`] and
    /// [`StageConstraint::After`] references from other stages. It must
    /// be unique within a single [`StageRegistry`](crate::registry::StageRegistry).
    fn name(&self) -> &str;

    /// Returns ordering constraints for this stage.
    ///
    /// Override this method to declare that this stage must run before or
    /// after another named stage. Return an empty `Vec` (the default) if
    /// there are no ordering requirements.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn constraints(&self) -> Vec<StageConstraint> {
    ///     vec![StageConstraint::After("logging".to_string())]
    /// }
    /// ```
    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        Vec::new()
    }

    /// Execute the stage with the given context.
    ///
    /// # Return values
    ///
    /// - `Ok(true)` -- continue to the next stage.
    /// - `Ok(false)` -- stop the pipeline gracefully (no error, remaining
    ///   stages are skipped).
    /// - `Err(PipelineError)` -- stop the pipeline with an error. Inverse
    ///   operations still run for stages that already executed.
    ///
    /// # Data sharing
    ///
    /// Use `ctx.extensions` (a type map) to store data during `run` that
    /// `run_inverse` will read later.
    async fn run(&self, ctx: &mut C) -> Result<bool, PipelineError>;

    /// Cleanup operation for this stage, called in reverse execution order.
    ///
    /// `run_inverse` is called after the forward pass completes (successfully
    /// or with error) for every stage whose `run` returned `Ok(true)` or
    /// `Ok(false)`. Stages whose `run` returned `Err` are not cleaned up.
    ///
    /// Use this to release resources, flush buffers, or inject response
    /// headers. If you stored data in `ctx.extensions` during `run`, retrieve
    /// it here (and remove it with `ctx.extensions.remove::<T>()`).
    #[inline]
    async fn run_inverse(&self, _ctx: &mut C) -> Result<(), PipelineError> {
        Ok(())
    }

    /// Returns whether this stage should be included in the pipeline for the
    /// given configuration block.
    ///
    /// Called once per stage when building a pipeline with
    /// [`StageRegistry::build_with_config`](crate::registry::StageRegistry::build_with_config).
    /// Return `false` to exclude the stage entirely (it will not appear in
    /// the pipeline at all). The default returns `true`.
    #[inline]
    fn is_applicable(&self, _config: Option<&ServerConfigurationBlock>) -> bool {
        true
    }
}

/// Observability hooks invoked around each stage during pipeline execution.
///
/// Implement this trait to instrument stage execution (e.g. emit per-stage
/// trace spans, log timing) without coupling the [`Pipeline`] to
/// observability code. Pass the hooks to
/// [`Pipeline::execute_with_hooks`].
///
/// All methods have default no-op implementations.
#[async_trait(?Send)]
pub trait StageHooks<C>: Send + Sync {
    /// Called before a stage's `run` method is invoked.
    #[inline]
    async fn before_stage(&mut self, _stage: &dyn Stage<C>) {}

    /// Called after a stage's `run` method completes.
    /// `result` is the outcome of `stage.run(ctx)`.
    #[inline]
    async fn after_stage(
        &mut self,
        _stage: &dyn Stage<C>,
        _result: &Result<bool, PipelineError>,
        _ctx: &mut C,
    ) {
    }

    /// Called before a stage's `run_inverse` method is invoked.
    #[inline]
    async fn before_stage_inverse(&mut self, _stage: &dyn Stage<C>) {}

    /// Called after a stage's `run_inverse` method completes.
    #[inline]
    async fn after_stage_inverse(
        &mut self,
        _stage: &dyn Stage<C>,
        _result: &Result<(), PipelineError>,
        _ctx: &mut C,
    ) {
    }
}

/// An ordered sequence of [`Stage`] instances to be executed.
///
/// Pipelines are built by [`StageRegistry::build_all`](crate::registry::StageRegistry::build_all)
/// or [`StageRegistry::build_with_config`](crate::registry::StageRegistry::build_with_config).
/// They execute stages in order and run inverse (cleanup) operations in
/// reverse order after the forward pass completes.
///
/// # Execution modes
///
/// | Method | Inverse pass | Hooks |
/// |---|---|---|
/// | [`execute`](Self::execute) | automatic | no |
/// | [`execute_with_hooks`](Self::execute_with_hooks) | automatic | yes |
/// | [`execute_without_inverse`](Self::execute_without_inverse) | manual | no |
/// | [`execute_without_inverse_with_hooks`](Self::execute_without_inverse_with_hooks) | manual | yes |
///
/// Use the `*_without_inverse` variants when you need fine-grained control
/// over when cleanup runs (e.g. to hold a lock across the entire pipeline).
#[derive(Clone, Default)]
pub struct Pipeline<C> {
    stages: Vec<Arc<dyn Stage<C>>>,
}

impl<C> Pipeline<C> {
    /// Create a new empty pipeline.
    ///
    /// Prefer building pipelines via
    /// [`StageRegistry::build_all`](crate::registry::StageRegistry::build_all)
    /// rather than constructing them manually.
    #[inline]
    pub fn new() -> Self {
        Self { stages: vec![] }
    }

    /// Add a stage to the end of the pipeline.
    ///
    /// The stage will execute after all previously added stages.
    #[inline]
    pub fn add_stage(mut self, stage: Arc<dyn Stage<C>>) -> Self {
        self.stages.push(stage);
        self
    }

    /// Execute the pipeline, running inverse operations in reverse order on completion.
    ///
    /// Stages execute in order until one returns `Ok(false)` or an error.
    /// After the forward pass, [`run_inverse`](Stage::run_inverse) is called
    /// for all executed stages in reverse order. If any `run_inverse` returns
    /// `Err`, execution stops immediately.
    #[inline]
    pub async fn execute(&self, ctx: &mut C) -> Result<(), PipelineError> {
        let mut executed_stages = Vec::with_capacity(self.stages.len());
        for stage in &self.stages {
            executed_stages.push(stage);
            match stage.run(ctx).await {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => return Err(e),
            }
        }

        // Execute inverse operations in reverse order, stopping on error
        for stage in executed_stages.iter().rev() {
            stage.run_inverse(ctx).await?;
        }
        Ok(())
    }

    /// Execute stages without running inverse operations, returning the executed stages.
    ///
    /// This gives you manual control over when cleanup runs. Use the returned
    /// stage list with [`execute_inverse`](Self::execute_inverse) later.
    #[inline]
    pub async fn execute_without_inverse<'a>(
        &'a self,
        ctx: &mut C,
    ) -> Result<Vec<&'a Arc<dyn Stage<C>>>, PipelineError> {
        let mut executed_stages = Vec::with_capacity(self.stages.len());
        for stage in &self.stages {
            executed_stages.push(stage);
            match stage.run(ctx).await {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(executed_stages)
    }

    /// Execute inverse (cleanup) operations for the given stages in reverse order.
    ///
    /// Pass the list returned by [`execute_without_inverse`](Self::execute_without_inverse).
    #[inline]
    pub async fn execute_inverse<'a>(
        &'a self,
        ctx: &mut C,
        executed_stages: Vec<&'a Arc<dyn Stage<C>>>,
    ) -> Result<(), PipelineError> {
        for stage in executed_stages.iter().rev() {
            stage.run_inverse(ctx).await?;
        }
        Ok(())
    }

    /// Execute inverse operations for the given stages with per-stage hooks.
    ///
    /// Same as [`execute_inverse`](Self::execute_inverse), but invokes
    /// [`StageHooks::before_stage_inverse`] and
    /// [`StageHooks::after_stage_inverse`] around each cleanup call.
    #[inline]
    pub async fn execute_inverse_with_hooks<'a, H: StageHooks<C>>(
        &'a self,
        ctx: &mut C,
        executed_stages: Vec<&'a Arc<dyn Stage<C>>>,
        hooks: &mut H,
    ) -> Result<(), PipelineError> {
        for stage in executed_stages.iter().rev() {
            hooks.before_stage_inverse(stage.as_ref()).await;
            let result = stage.run_inverse(ctx).await;
            hooks
                .after_stage_inverse(stage.as_ref(), &result, ctx)
                .await;
            result?;
        }
        Ok(())
    }

    /// Execute the pipeline with per-stage hooks, running inverse operations in reverse order.
    ///
    /// Behaves identically to [`execute`](Self::execute), but invokes the
    /// provided `hooks` before and after each stage's `run` and `run_inverse`
    /// methods. This allows callers to instrument stage execution (e.g., emit
    /// per-stage trace spans) without coupling the Pipeline to observability code.
    #[inline]
    pub async fn execute_with_hooks<H: StageHooks<C>>(
        &self,
        ctx: &mut C,
        hooks: &mut H,
    ) -> Result<(), PipelineError> {
        let mut executed_stages = Vec::with_capacity(self.stages.len());
        for stage in &self.stages {
            hooks.before_stage(stage.as_ref()).await;
            let result = stage.run(ctx).await;
            hooks.after_stage(stage.as_ref(), &result, ctx).await;
            executed_stages.push(stage);
            match result {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => return Err(e),
            }
        }

        // Execute inverse operations in reverse order, stopping on error
        for stage in executed_stages.iter().rev() {
            hooks.before_stage_inverse(stage.as_ref()).await;
            let result = stage.run_inverse(ctx).await;
            hooks
                .after_stage_inverse(stage.as_ref(), &result, ctx)
                .await;
            result?;
        }
        Ok(())
    }

    /// Execute stages without running inverse operations, with per-stage hooks.
    ///
    /// Behaves identically to [`execute_without_inverse`](Self::execute_without_inverse),
    /// but invokes the provided `hooks` before and after each stage's `run` method.
    #[inline]
    pub async fn execute_without_inverse_with_hooks<'a, H: StageHooks<C>>(
        &'a self,
        ctx: &mut C,
        hooks: &mut H,
    ) -> Result<Vec<&'a Arc<dyn Stage<C>>>, PipelineError> {
        let mut executed_stages = Vec::with_capacity(self.stages.len());
        for stage in &self.stages {
            hooks.before_stage(stage.as_ref()).await;
            let result = stage.run(ctx).await;
            hooks.after_stage(stage.as_ref(), &result, ctx).await;
            executed_stages.push(stage);
            match result {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(executed_stages)
    }
}
