use std::sync::Arc;

use ferron_core::{
    config::{validator::ConfigurationValidator, ServerConfigurationValue},
    config_validator_scoped_key,
    loader::ModuleLoader,
    providers::Provider,
    validate_directive,
};
use ferron_observability::{
    AccessVisitor, ApplicationLogFormatterContext, LogAttributeValue, LogFormatterContext, LogLevel,
};
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

struct JsonApplicationFormatObservabilityProvider;

impl Provider<ApplicationLogFormatterContext<'static>>
    for JsonApplicationFormatObservabilityProvider
{
    fn name(&self) -> &str {
        "json"
    }

    fn execute(
        &self,
        ctx: &mut ApplicationLogFormatterContext<'static>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut json_attributes_map = serde_json::Map::new();
        ctx.log_event.attributes.iter().for_each(|(k, v)| {
            json_attributes_map.insert(
                k.to_string(),
                match v {
                    LogAttributeValue::Bool(b) => serde_json::json!(b),
                    LogAttributeValue::String(s) => serde_json::json!(s),
                    LogAttributeValue::StaticStr(s) => serde_json::json!(s),
                    LogAttributeValue::I64(i) => serde_json::json!(i),
                    LogAttributeValue::F64(f) => serde_json::json!(f),
                },
            );
        });

        let mut json_map = serde_json::Map::new();
        json_map.insert(
            "timestamp".to_string(),
            serde_json::json!(&std::time::UNIX_EPOCH.elapsed().map(|d| d.as_millis()).ok()),
        );
        json_map.insert(
            "level".to_string(),
            serde_json::json!(match ctx.log_event.level {
                LogLevel::Error => "ERROR",
                LogLevel::Warn => "WARN",
                LogLevel::Info => "INFO",
                LogLevel::Debug => "DEBUG",
            }),
        );
        json_map.insert(
            "summary".to_string(),
            serde_json::json!(&ctx.log_event.summary),
        );
        json_map.insert(
            "target".to_string(),
            serde_json::json!(&ctx.log_event.target),
        );
        json_map.insert(
            "attributes".to_string(),
            serde_json::Value::Object(json_attributes_map),
        );

        if let Some(trace_context) = &ctx.log_event.trace_context {
            json_map.insert(
                "trace_context".to_string(),
                serde_json::json!({
                    "trace_id": std::str::from_utf8(&trace_context.trace_id).ok(),
                    "span_id": std::str::from_utf8(&trace_context.span_id).ok(),
                    "sampled": trace_context.sampled
                }),
            );
        } else {
            json_map.insert("trace_context".to_string(), serde_json::Value::Null);
        }

        ctx.output = Some(serde_json::Value::Object(json_map).to_string());

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
            .with_provider::<ApplicationLogFormatterContext<'static>, _>(|| {
                Arc::new(JsonApplicationFormatObservabilityProvider)
            })
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ferron_core::config::validator::ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("logformat", "json"),
            Box::new(JsonFormatLogFormatConfigurationValidator),
        );
        registry.insert(
            config_validator_scoped_key!("logformat_application", "json"),
            Box::new(JsonFormatLogApplicationFormatConfigurationValidator),
        );
    }
}

struct JsonFormatLogFormatConfigurationValidator;

impl ConfigurationValidator for JsonFormatLogFormatConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_directive!(config, validator_ctx.used_directives, fields, optional args(*) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});

        Ok(())
    }
}

struct JsonFormatLogApplicationFormatConfigurationValidator;

impl ConfigurationValidator for JsonFormatLogApplicationFormatConfigurationValidator {
    fn validate_block(
        &self,
        _config: &ferron_core::config::ServerConfigurationBlock,
        _validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}
