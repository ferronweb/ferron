//! Pipeline execution framework with ordered stages and inverse operations.
//!
//! Pipelines execute stages sequentially, with support for early termination
//! and inverse operations (like cleanup) in reverse order.

use async_trait::async_trait;
use std::sync::Arc;

use crate::StageConstraint;

/// Error type for pipeline execution failures.
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

/// A stage in the execution pipeline.
///
/// Stages are ordered components that execute sequentially. They can:
/// - Continue to the next stage (return `Ok(true)`)
/// - Stop the pipeline gracefully (return `Ok(false)`)
/// - Terminate with an error (return `Err`)
/// - Define ordering constraints (Before/After other stages)
/// - Optionally run inverse operations for cleanup
#[async_trait(?Send)]
pub trait Stage<C>: Send + Sync {
    /// Returns the name of this stage (used for ordering constraints).
    fn name(&self) -> &str;

    /// Returns ordering constraints for this stage.
    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        Vec::new()
    }

    /// Execute the stage with the given context.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` - Continue to next stage
    /// - `Ok(false)` - Terminate pipeline gracefully
    /// - `Err` - Terminate pipeline with error
    async fn run(&self, ctx: &mut C) -> Result<bool, PipelineError>;

    /// Inverse operation for this stage (e.g., cleanup).
    ///
    /// Called during pipeline finalization or error handling in reverse execution order.
    /// Only called for stages that successfully executed.
    #[inline]
    async fn run_inverse(&self, _ctx: &mut C) -> Result<(), PipelineError> {
        Ok(())
    }
}

/// Hooks invoked around each stage during pipeline execution.
///
/// Implement this trait to observe or instrument stage execution (e.g., emit
/// per-stage trace spans) without coupling the Pipeline to observability code.
#[async_trait(?Send)]
pub trait StageHooks<C>: Send + Sync {
    /// Called before a stage's `run` method is invoked.
    #[inline]
    async fn before_stage(&mut self, _stage: &dyn Stage<C>) {}

    /// Called after a stage's `run` method completes.
    /// `result` is the outcome of `stage.run(ctx)`.
    #[inline]
    async fn after_stage(&mut self, _stage: &dyn Stage<C>, _result: &Result<bool, PipelineError>) {}

    /// Called before a stage's `run_inverse` method is invoked.
    #[inline]
    async fn before_stage_inverse(&mut self, _stage: &dyn Stage<C>) {}

    /// Called after a stage's `run_inverse` method completes.
    #[inline]
    async fn after_stage_inverse(
        &mut self,
        _stage: &dyn Stage<C>,
        _result: &Result<(), PipelineError>,
    ) {
    }
}

/// An ordered sequence of stages to be executed.
///
/// Pipelines execute stages in order and support early termination.
/// After all stages complete, inverse operations are run in reverse order.
#[derive(Clone, Default)]
pub struct Pipeline<C> {
    stages: Vec<Arc<dyn Stage<C>>>,
}

impl<C> Pipeline<C> {
    /// Create a new empty pipeline.
    #[inline]
    pub fn new() -> Self {
        Self { stages: vec![] }
    }

    /// Add a stage to the end of the pipeline.
    #[inline]
    pub fn add_stage(mut self, stage: Arc<dyn Stage<C>>) -> Self {
        self.stages.push(stage);
        self
    }

    /// Execute the pipeline, running inverse operations in reverse order on completion.
    ///
    /// Stages are executed in order until one returns `Ok(false)` or an error.
    /// After execution completes (successfully or with error), inverse operations
    /// are run for all executed stages in reverse order.
    pub async fn execute(&self, ctx: &mut C) -> Result<(), PipelineError> {
        let mut executed_stages = vec![];
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

    /// Execute stages without running inverse operations, returning executed stages.
    ///
    /// This allows manual control over when inverse operations are run.
    /// Use with `execute_inverse` to separate stage execution from cleanup.
    pub async fn execute_without_inverse<'a>(
        &'a self,
        ctx: &mut C,
    ) -> Result<Vec<&'a Arc<dyn Stage<C>>>, PipelineError> {
        let mut executed_stages = vec![];
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

    /// Execute inverse operations for the given stages in reverse order.
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
    pub async fn execute_inverse_with_hooks<'a, H: StageHooks<C>>(
        &'a self,
        ctx: &mut C,
        executed_stages: Vec<&'a Arc<dyn Stage<C>>>,
        hooks: &mut H,
    ) -> Result<(), PipelineError> {
        for stage in executed_stages.iter().rev() {
            hooks.before_stage_inverse(stage.as_ref()).await;
            let result = stage.run_inverse(ctx).await;
            hooks.after_stage_inverse(stage.as_ref(), &result).await;
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
    pub async fn execute_with_hooks<H: StageHooks<C>>(
        &self,
        ctx: &mut C,
        hooks: &mut H,
    ) -> Result<(), PipelineError> {
        let mut executed_stages = vec![];
        for stage in &self.stages {
            hooks.before_stage(stage.as_ref()).await;
            let result = stage.run(ctx).await;
            hooks.after_stage(stage.as_ref(), &result).await;
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
            hooks.after_stage_inverse(stage.as_ref(), &result).await;
            result?;
        }
        Ok(())
    }

    /// Execute stages without running inverse operations, with per-stage hooks.
    ///
    /// Behaves identically to [`execute_without_inverse`](Self::execute_without_inverse),
    /// but invokes the provided `hooks` before and after each stage's `run` method.
    pub async fn execute_without_inverse_with_hooks<'a, H: StageHooks<C>>(
        &'a self,
        ctx: &mut C,
        hooks: &mut H,
    ) -> Result<Vec<&'a Arc<dyn Stage<C>>>, PipelineError> {
        let mut executed_stages = vec![];
        for stage in &self.stages {
            hooks.before_stage(stage.as_ref()).await;
            let result = stage.run(ctx).await;
            hooks.after_stage(stage.as_ref(), &result).await;
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
