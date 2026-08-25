//! External-command DNS provider for the ACME DNS-01 challenge.
//!
//! The `command` provider runs an external program for every DNS record change
//! and passes the record details through environment variables. The program must
//! exit with status `0` to signal success. This lets operators delegate DNS
//! updates to any DNS server or automation that Ferron does not support natively.

mod client;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::validator::{
    ConfigurationValidationError, ConfigurationValidator, ConfigurationValidatorContext,
    ConfigurationValidatorScopedKey,
};
use ferron_core::config::ServerConfigurationBlock;
use ferron_core::config_validator_scoped_key;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_core::registry::RegistryBuilder;
use ferron_dns::DnsContext;

pub struct CommandDnsModuleLoader;

impl ModuleLoader for CommandDnsModuleLoader {
    fn register_providers(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_provider::<DnsContext<'static>, _>(|| Arc::new(CommandDnsProvider))
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut HashMap<ConfigurationValidatorScopedKey, Box<dyn ConfigurationValidator>>,
    ) {
        registry.insert(
            config_validator_scoped_key!("dns", "command"),
            Box::new(CommandDnsConfigurationValidator),
        );
    }
}

struct CommandDnsProvider;

impl Provider<DnsContext<'static>> for CommandDnsProvider {
    fn name(&self) -> &str {
        "command"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let program = ctx
            .config
            .get_value("command")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or_else(|| {
                anyhow::anyhow!("Missing or invalid `command` for 'command' DNS provider")
            })?;

        let min_ttl = ctx
            .config
            .get_value("min_ttl")
            .and_then(|v| v.as_number())
            .filter(|v| *v >= 0)
            .map(|v| v as u32)
            .unwrap_or(60);

        ctx.client = Some(Arc::new(client::CommandDnsClient::new(program, min_ttl)));
        Ok(())
    }
}

struct CommandDnsConfigurationValidator;

impl ConfigurationValidator for CommandDnsConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        validator_ctx: &mut ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let has_command = config
            .get_value("command")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .is_some();

        validator_ctx.used_directives.insert("command".to_string());
        validator_ctx.used_directives.insert("min_ttl".to_string());

        if !has_command {
            return Err(ConfigurationValidationError::from(anyhow::anyhow!(
                "Missing or invalid `command` for 'command' DNS provider"
            )));
        }

        let command_value = config
            .get_value("command")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        if let Some(command) = command_value {
            if !command.starts_with('/') {
                validator_ctx.add_best_practice_violation(
                    "The `command` DNS provider executes an external program; prefer an absolute, trusted path to avoid running untrusted binaries from PATH.",
                    None,
                );
            }
        }

        Ok(())
    }
}
