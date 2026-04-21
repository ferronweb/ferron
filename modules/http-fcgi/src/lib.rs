use std::sync::{atomic::AtomicUsize, Arc};

use cegla_fcgi::client::SendRequest;
use ferron_http::{HttpContext, HttpFileContext};
use http_body_util::combinators::UnsyncBoxBody;
use parking_lot::RwLock;

use crate::stages::{FcgiFileStage, FcgiPassStage};

mod client;
mod config;
mod stages;
mod util;
mod validator;

const DEFAULT_CONCURRENT_CONNECTIONS: usize = 16384;

static GLOBAL_CONCURRENT_CONNECTIONS: AtomicUsize =
    AtomicUsize::new(DEFAULT_CONCURRENT_CONNECTIONS);

/// Body type used for FastCGI requests.
pub type ProxyBody = UnsyncBoxBody<bytes::Bytes, std::io::Error>;

/// Connection pool item containing the HTTP client.
pub type ConnpoolItem = connpool::Item<String, SendRequest<ProxyBody>>;

pub struct FcgiModuleLoader {
    client: RwLock<client::FcgiClient>,
}

impl Default for FcgiModuleLoader {
    fn default() -> Self {
        Self {
            client: RwLock::new(client::FcgiClient::new(DEFAULT_CONCURRENT_CONNECTIONS)),
        }
    }
}

impl ferron_core::loader::ModuleLoader for FcgiModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(validator::FcgiConfigurationValidator));
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
            .push(Box::new(validator::FcgiConfigurationValidator));
    }

    fn register_stages(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        let client = Arc::new(self.client.read().clone());
        let client2 = client.clone();
        registry
            .with_stage::<HttpContext, _>(move || Arc::new(FcgiPassStage::new(client.clone())))
            .with_stage::<HttpFileContext, _>(move || Arc::new(FcgiFileStage::new(client2.clone())))
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
            .get("fcgi_concurrent_conns")
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
