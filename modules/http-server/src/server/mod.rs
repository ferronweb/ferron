//! HTTP server implementation

use std::sync::Arc;

use ferron_core::pipeline::Pipeline;
use ferron_core::runtime::Runtime;
use ferron_core::Module;
use ferron_http::HttpContext;
use parking_lot::Mutex;

use crate::config::{prepare_host_config, ThreeStageResolver};

mod tcp;

pub struct BasicHttpModule {
    pipeline: Arc<Pipeline<HttpContext>>,
    global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    config_resolver: Arc<crate::config::ThreeStageResolver>,
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
        // TODO: TLS resolver
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
