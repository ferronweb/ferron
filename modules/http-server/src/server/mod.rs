//! HTTP server implementation

use std::sync::Arc;

use ferron_core::runtime::Runtime;
use ferron_core::Module;
use ferron_core::{config::ServerConfigurationBlock, pipeline::Pipeline};
use ferron_http::HttpContext;
use ferron_tls::TcpTlsContext;
use parking_lot::Mutex;

use crate::{
    config::{prepare_host_config, ThreeStageResolver},
    server::tls_resolve::TlsResolverRadixTree,
};

mod tcp;
mod tls_resolve;

pub struct BasicHttpModule {
    pipeline: Arc<Pipeline<HttpContext>>,
    global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    config_resolver: Arc<crate::config::ThreeStageResolver>,
    tls_resolver: Option<Arc<self::tls_resolve::TlsResolverRadixTree>>,
    listeners: Mutex<Vec<tcp::TcpListenerHandle>>,
    port: u16,
}

impl BasicHttpModule {
    pub fn new(
        registry: &ferron_core::registry::Registry,
        port_config: ferron_core::config::ServerConfigurationPort,
        global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
        port: u16,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // TODO: insert specific TLS resolvers
        let mut enable_tls = false;
        let mut tls_resolver = TlsResolverRadixTree::new();
        for host_config in &port_config.hosts {
            if let Some(tls) = host_config.1.directives.get("tls") {
                for tls1 in tls {
                    // TODO: implicit automatic TLS
                    if tls1
                        .args
                        .first()
                        .map(|a| a.as_boolean().unwrap_or(true))
                        .unwrap_or(false)
                    {
                        enable_tls = true;
                        let tls_provider_name = tls1
                            .children
                            .as_ref()
                            .and_then(|c| c.get_value("provider"))
                            .ok_or(anyhow::anyhow!("TLS provider not specified"))?
                            .as_str()
                            .ok_or(anyhow::anyhow!("TLS provider must be a string"))?;

                        if let Some(tls_registry) =
                            registry.get_provider_registry::<TcpTlsContext>()
                        {
                            let tls_provider = tls_registry
                                .get(tls_provider_name)
                                .ok_or(anyhow::anyhow!("TLS provider not found"))?;

                            let mut tls_resolver_ctx = TcpTlsContext {
                                // SAFETY: We know that the lifetime of the config is longer
                                //         than the lifetime of the resolver. but "'static"
                                //         is the only lifetime we can use here. This
                                //         constraint is enforced by the provider registry.
                                config: unsafe {
                                    std::mem::transmute::<
                                        &ServerConfigurationBlock,
                                        &'static ServerConfigurationBlock,
                                    >(
                                        &tls1.children.as_ref().expect("TLS config not found")
                                    )
                                },
                                resolver: None,
                            };
                            tls_provider.execute(&mut tls_resolver_ctx)?;
                            let tls_resolver_sub = tls_resolver_ctx
                                .resolver
                                .ok_or(anyhow::anyhow!("TLS resolver not found"))?;

                            match (&host_config.0.host, host_config.0.ip) {
                                (Some(host), Some(ip)) => {
                                    tls_resolver.insert_ip_and_hostname(
                                        ip,
                                        host,
                                        tls_resolver_sub,
                                        false,
                                    );
                                }
                                (Some(host), None) => {
                                    tls_resolver.insert_hostname(host, tls_resolver_sub, false);
                                }
                                (None, Some(ip)) => {
                                    tls_resolver.insert_ip(ip, tls_resolver_sub);
                                }
                                (None, None) => {
                                    // Ignore this case,
                                    // as it is not possible to have a host config without a host or ip
                                }
                            }
                        }
                    }
                }
            }
        }
        let pipeline = registry
            .get_stage_registry::<HttpContext>()
            .expect("HTTP stage registry not found")
            .build_all();
        Ok(Self {
            pipeline: Arc::new(pipeline),
            global_config,
            config_resolver: Arc::new(ThreeStageResolver::from_prepared(prepare_host_config(
                port_config,
            )?)),
            tls_resolver: if enable_tls {
                Some(Arc::new(tls_resolver))
            } else {
                None
            },
            listeners: Mutex::new(Vec::new()),
            port,
        })
    }
}

impl Module for BasicHttpModule {
    fn name(&self) -> &str {
        "http"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        let ports = if self.port != 0 {
            vec![self.port]
        } else {
            vec![80]
        };
        for port in ports {
            let pipeline = self.pipeline.clone();
            let listener = tcp::TcpListenerHandle::new(port, pipeline)?;
            self.listeners.lock().push(listener);
            // TODO: QUIC
        }

        Ok(())
    }
}

impl Drop for BasicHttpModule {
    fn drop(&mut self) {
        for listener in &*self.listeners.lock() {
            listener.cancel();
        }
    }
}
