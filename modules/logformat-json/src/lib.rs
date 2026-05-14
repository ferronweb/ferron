use std::sync::Arc;

use ferron_core::{loader::ModuleLoader, providers::Provider};
use ferron_observability::{AccessVisitor, LogFormatterContext};
use serde_json::{Map, Value};

struct JsonVisitor {
    inner: Map<String, Value>,
    /// Field names to include in output. Empty means all fields.
    enabled_fields: Arc<Vec<String>>,
}

impl AccessVisitor for JsonVisitor {
    fn field_string(&mut self, name: &str, value: &str) {
        if self.is_enabled(name) {
            self.inner
                .insert(name.to_string(), Value::String(value.to_string()));
        }
    }

    fn field_u64(&mut self, name: &str, value: u64) {
        if self.is_enabled(name) {
            self.inner
                .insert(name.to_string(), Value::Number(value.into()));
        }
    }

    fn field_f64(&mut self, name: &str, value: f64) {
        if self.is_enabled(name) {
            if let Some(n) = serde_json::Number::from_f64(value) {
                self.inner.insert(name.to_string(), Value::Number(n));
            }
        }
    }

    fn field_bool(&mut self, name: &str, value: bool) {
        if self.is_enabled(name) {
            self.inner.insert(name.to_string(), Value::Bool(value));
        }
    }
}

impl JsonVisitor {
    fn is_enabled(&self, name: &str) -> bool {
        self.enabled_fields.is_empty() || self.enabled_fields.iter().any(|f| f == name)
    }
}

fn parse_enabled_fields(
    log_config: &ferron_core::config::ServerConfigurationBlock,
) -> Arc<Vec<String>> {
    let fields = log_config
        .directives
        .get("fields")
        .map(|entries| {
            entries
                .iter()
                .flat_map(|entry| entry.args.iter())
                .filter_map(|arg| {
                    arg.as_string_with_interpolations(&std::collections::HashMap::new())
                })
                .collect()
        })
        .unwrap_or_default();
    Arc::new(fields)
}

struct JsonFormatObservabilityProvider;

impl Provider<LogFormatterContext> for JsonFormatObservabilityProvider {
    fn name(&self) -> &str {
        "json"
    }

    fn execute(&self, ctx: &mut LogFormatterContext) -> Result<(), Box<dyn std::error::Error>> {
        let enabled_fields = parse_enabled_fields(&ctx.log_config);
        let mut visitor = JsonVisitor {
            inner: Default::default(),
            enabled_fields,
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
