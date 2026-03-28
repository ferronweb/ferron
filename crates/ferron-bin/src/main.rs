use ferron_core::http::HttpContext;
use ferron_http::BasicHttpModule;
use ferron_module_api::{FerronModule, HttpModule};
use ferron_runtime::pipeline::Pipeline;

#[tokio::main]
async fn main() {
    // =====================
    // Build HTTP pipeline
    // =====================

    let mut pipeline = Pipeline::<HttpContext>::new();

    // You can still use modules to build pipeline
    let http_module_builder = ferron_http::BasicHttpModuleBuilder;

    pipeline = http_module_builder.register(pipeline);

    // =====================
    // Instantiate modules
    // =====================

    let modules: Vec<Box<dyn FerronModule>> = vec![Box::new(BasicHttpModule::new(pipeline))];

    // =====================
    // Start servers
    // =====================

    let mut handles = vec![];

    for module in &modules {
        if let Some(server) = module.server() {
            handles.push(tokio::spawn(server.start()));
        }
    }

    // Wait forever
    futures_util::future::join_all(handles).await;
}
