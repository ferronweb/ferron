use std::sync::Arc;

use ferron_core::{loader::ModuleLoader, providers::Provider};
use ferron_observability::{AccessVisitor, LogFormatterContext};

struct JsonVisitor {
    inner: serde_json::Map<String, serde_json::Value>,
}

impl AccessVisitor for JsonVisitor {
    fn field_string(&mut self, name: &str, value: &str) {
        self.inner.insert(
            name.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }

    fn field_u64(&mut self, name: &str, value: u64) {
        self.inner
            .insert(name.to_string(), serde_json::Value::Number(value.into()));
    }

    fn field_f64(&mut self, name: &str, value: f64) {
        if let Some(n) = serde_json::Number::from_f64(value) {
            self.inner
                .insert(name.to_string(), serde_json::Value::Number(n));
        }
    }

    fn field_bool(&mut self, name: &str, value: bool) {
        self.inner
            .insert(name.to_string(), serde_json::Value::Bool(value));
    }
}

struct JsonFormatObservabilityProvider;

impl Provider<LogFormatterContext> for JsonFormatObservabilityProvider {
    fn name(&self) -> &str {
        "json"
    }

    fn execute(&self, ctx: &mut LogFormatterContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut visitor = JsonVisitor {
            inner: Default::default(),
        };
        ctx.access_event.visit(&mut visitor);
        ctx.output = Some(serde_json::to_string(&visitor.inner)?);
        Ok(())
    }
}

pub struct JsonFormatObservabilityModuleLoader;

impl ModuleLoader for JsonFormatObservabilityModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry
            .with_provider::<LogFormatterContext, _>(|| Arc::new(JsonFormatObservabilityProvider))
    }
}
