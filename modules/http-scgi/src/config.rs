use std::collections::HashMap;

use ferron_http::HttpContext;

/*
  scgi {
    backend "tcp://127.0.0.1:4000/"
    environment "A" "B"
  }
*/

pub struct ScgiConfiguration {
    pub backend_server: String,
    pub environment: HashMap<String, String>,
}

impl ScgiConfiguration {
    pub fn from_http_ctx(ctx: &HttpContext) -> Option<Self> {
        let cgi_config = ctx.configuration.get_entry("scgi", false)?;
        let mut backend_server = cgi_config
            .get_value()
            .and_then(|v| v.as_string_with_interpolations(ctx));
        if backend_server.is_none() && !cgi_config.get_flag() {
            return None;
        }
        let Some(cgi_children) = cgi_config.children.as_ref() else {
            return Some(ScgiConfiguration {
                backend_server: backend_server?,
                environment: HashMap::new(),
            });
        };

        if backend_server.is_none() {
            backend_server = cgi_children
                .get_value("backend")
                .and_then(|v| v.as_string_with_interpolations(ctx));
        }

        let mut environment = HashMap::new();
        if let Some(entries) = cgi_children.directives.get("environment") {
            for entry in entries {
                let mut args_iter = entry.args.iter();
                if let Some(name) = args_iter.next().and_then(|v| v.as_str()) {
                    if let Some(value) = args_iter
                        .next()
                        .and_then(|v| v.as_string_with_interpolations(ctx))
                    {
                        environment.insert(name.to_string(), value.to_string());
                    }
                }
            }
        }

        Some(ScgiConfiguration {
            backend_server: backend_server?,
            environment,
        })
    }
}
