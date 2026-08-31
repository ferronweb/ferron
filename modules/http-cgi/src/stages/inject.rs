use std::sync::Arc;

use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_http::HttpContext;
use rustc_hash::FxHashMap;

use crate::config::CgiConfiguration;

pub struct CgiInjectStage;

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for CgiInjectStage {
    fn name(&self) -> &str {
        "cgi_inject"
    }

    fn constraints(&self) -> Vec<ferron_core::registry::StageConstraint> {
        vec![
            ferron_core::registry::StageConstraint::Before("reverse_proxy".to_string()),
            ferron_core::registry::StageConstraint::After("forward_proxy".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|b| b.has_directive("cgi"))
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let Some(config) = CgiConfiguration::from_http_ctx(ctx) else {
            // CGI not configured
            return Ok(true);
        };

        if ctx.configuration.get_entry("index", false).is_none() {
            // Inject default index extensions
            let mut index_inject_ext = vec![
                "index.html".to_string(),
                "index.htm".to_string(),
                "index.xhtml".to_string(),
            ];
            if config.additional_extensions.contains(".cgi") {
                index_inject_ext.insert(0, "index.cgi".to_string());
            }
            if config.additional_extensions.contains(".php") {
                index_inject_ext.insert(0, "index.php".to_string());
            }
            if index_inject_ext.len() > 3 {
                let mut directives = FxHashMap::default();
                directives.insert(
                    "index".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: index_inject_ext
                            .into_iter()
                            .map(|s| ServerConfigurationValue::String(s, None))
                            .collect(),
                        children: None,
                        span: None,
                    }],
                );
                let block = ServerConfigurationBlock {
                    directives: Arc::new(directives),
                    matchers: FxHashMap::default(),
                    span: None,
                };
                ctx.configuration.add_layer(Arc::new(block));
            }
        }

        Ok(true)
    }
}
