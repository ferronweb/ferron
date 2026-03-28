use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait Stage<C>: Send + Sync {
    async fn run(&self, ctx: &mut C);
}

pub struct Pipeline<C> {
    stages: Vec<Arc<dyn Stage<C>>>,
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
