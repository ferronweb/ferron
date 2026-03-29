use async_trait::async_trait;
use std::sync::Arc;

use crate::StageConstraint;

/// Error type for pipeline execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    /// Stage requested pipeline termination
    Terminated,
    /// Custom error from a stage
    Custom(String),
}

impl PipelineError {
    pub fn custom(msg: impl Into<String>) -> Self {
        PipelineError::Custom(msg.into())
    }
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineError::Terminated => write!(f, "pipeline terminated by stage"),
            PipelineError::Custom(msg) => write!(f, "pipeline error: {}", msg),
        }
    }
}

impl std::error::Error for PipelineError {}

#[async_trait]
pub trait Stage<C>: Send + Sync {
    /// Returns the name of this stage
    fn name(&self) -> &str;

    /// Returns the ordering constraints for this stage
    fn constraints(&self) -> Vec<StageConstraint> {
        Vec::new()
    }

    /// Execute the stage with the given context
    /// Returns Ok(true) to continue pipeline, Ok(false) to terminate gracefully
    /// Returns Err to terminate with an error
    async fn run(&self, ctx: &mut C) -> Result<bool, PipelineError>;

    /// Inverse operation for this stage
    /// Returns Err to terminate the inverse operation
    async fn run_inverse(&self, _ctx: &mut C) -> Result<(), PipelineError> {
        Ok(())
    }
}

pub struct Pipeline<C> {
    stages: Vec<Arc<dyn Stage<C>>>,
}

impl<C> Clone for Pipeline<C> {
    fn clone(&self) -> Self {
        Self {
            stages: self.stages.clone(),
        }
    }
}

impl<C> Pipeline<C> {
    pub fn new() -> Self {
        Self { stages: vec![] }
    }

    pub fn add_stage(mut self, stage: Arc<dyn Stage<C>>) -> Self {
        self.stages.push(stage);
        self
    }

    /// Execute the pipeline, stopping early if a stage returns Ok(false) or Err
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
}
