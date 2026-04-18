use std::sync::{atomic::AtomicUsize, Arc};

use ferron_http::HttpContext;
use http_body_util::combinators::UnsyncBoxBody;
use parking_lot::RwLock;

use crate::client::ForwardedAuthClient;
use crate::validator::ForwardedAuthenticationConfigurationValidator;

pub use client::SendRequestWrapper;
mod client;
mod config;
mod stage;
mod validator;

const DEFAULT_CONCURRENT_CONNECTIONS: usize = 16384;
const DEFAULT_KEEPALIVE_IDLE_TIMEOUT: u64 = 60000;

static GLOBAL_CONCURRENT_CONNECTIONS: AtomicUsize =
    AtomicUsize::new(DEFAULT_CONCURRENT_CONNECTIONS);

/// Body type used for forwarded auth requests.
pub type ProxyBody = UnsyncBoxBody<bytes::Bytes, std::io::Error>;

/// Connection pool key for forwarded authentication.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct ConnpoolKey {
    pub url: String,
    pub unix_socket: Option<String>,
}

/// Connection pool item containing the HTTP client.
pub type ConnpoolItem = connpool::Item<ConnpoolKey, ConnpoolItemInner>;

/// Inner connection pool item.
pub struct ConnpoolItemInner {
    pub client: SendRequestWrapper,
    pub is_unix: bool,
}

pub struct ForwardedAuthenticationModuleLoader {
    client: RwLock<ForwardedAuthClient>,
}

impl Default for ForwardedAuthenticationModuleLoader {
    fn default() -> Self {
        Self {
            client: RwLock::new(ForwardedAuthClient::new(DEFAULT_CONCURRENT_CONNECTIONS)),
        }
    }
}

impl ferron_core::loader::ModuleLoader for ForwardedAuthenticationModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(ForwardedAuthenticationConfigurationValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            &'static str,
            Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(ForwardedAuthenticationConfigurationValidator));
    }

    fn register_stages(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        let client = Arc::new(self.client.read().clone());
        registry.with_stage::<HttpContext, _>(move || {
            Arc::new(stage::ForwardedAuthenticationStage::new(client.clone()))
        })
    }

    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        _modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(val) = config
            .global_config
            .directives
            .get("auth_to_concurrent_conns")
            .and_then(|entries| entries.first())
            .and_then(|e| e.args.first())
            .and_then(|v: &ferron_core::config::ServerConfigurationValue| v.as_number())
        {
            if val > 0 {
                let new_limit = val as usize;
                let old_limit =
                    GLOBAL_CONCURRENT_CONNECTIONS.load(std::sync::atomic::Ordering::Relaxed);
                GLOBAL_CONCURRENT_CONNECTIONS
                    .store(new_limit, std::sync::atomic::Ordering::Relaxed);

                if old_limit != new_limit {
                    self.client.write().update_global_limit(new_limit);
                }
            }
        }

        Ok(())
    }
}
