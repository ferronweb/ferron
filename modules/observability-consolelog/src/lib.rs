use std::sync::Arc;

use ferron_core::{
    loader::ModuleLoader, log_debug, log_error, log_info, log_warn, providers::Provider,
};
use ferron_observability::ObservabilityContext;

struct ConsoleObservabilityProvider;

impl Provider<ObservabilityContext> for ConsoleObservabilityProvider {
    fn name(&self) -> &str {
        "console"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn std::error::Error>> {
        // TODO: offload the logging into a secondary runtime to avoid blocking the main runtime
        match &ctx.event {
            ferron_observability::Event::Access(ae) => log_info!("{}", ae.message),
            ferron_observability::Event::Log(le) => match le.level {
                ferron_observability::LogLevel::Error => log_error!("{}", le.message),
                ferron_observability::LogLevel::Warn => log_warn!("{}", le.message),
                ferron_observability::LogLevel::Info => log_info!("{}", le.message),
                ferron_observability::LogLevel::Debug => log_debug!("{}", le.message),
            },
            _ => (), // Ignore unsupported event types
        };

        Ok(())
    }
}

pub struct ConsoleObservabilityModuleLoader;

impl ModuleLoader for ConsoleObservabilityModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry.with_provider::<ObservabilityContext, _>(|| Arc::new(ConsoleObservabilityProvider))
    }
}
