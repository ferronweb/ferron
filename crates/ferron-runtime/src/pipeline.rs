use async_trait::async_trait;
use std::sync::Arc;

use crate::StageConstraint;

#[async_trait]
pub trait Stage<C>: Send + Sync {
    /// Returns the name of this stage
    fn name(&self) -> &str;

    /// Returns the ordering constraints for this stage
    fn constraints(&self) -> Vec<StageConstraint> {
        Vec::new()
    }

    /// Execute the stage with the given context
    async fn run(&self, ctx: &mut C);
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

    pub async fn execute(&self, ctx: &mut C) {
        for stage in &self.stages {
            stage.run(ctx).await;
        }
    }
}
