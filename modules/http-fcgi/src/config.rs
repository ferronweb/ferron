use std::collections::{HashMap, HashSet};

use ferron_core::config::ServerConfigurationDirectiveEntry;
use ferron_http::HttpContext;

/*
  fcgi {
    backend "tcp://127.0.0.1:4000/"
    environment "A" "B"
    pass true
    keepalive false
  }
*/

pub struct FcgiConfiguration {
    pub extensions: HashSet<String>,
    pub backend_server: String,
    pub environment: HashMap<String, String>,
    pub pass: bool,
    pub keepalive: bool,
    pub local_limit: Option<usize>,
}

impl FcgiConfiguration {
    fn from_config_entry(
        cgi_config: &ServerConfigurationDirectiveEntry,
        is_fcgi_php: bool,
        ctx: &HttpContext,
    ) -> Option<Self> {
        let mut backend_server = cgi_config
            .get_value()
            .and_then(|v| v.as_string_with_interpolations(ctx));
        if backend_server.is_none() && !cgi_config.get_flag() {
            return None;
        }
        let Some(cgi_children) = cgi_config.children.as_ref() else {
            return Some(FcgiConfiguration {
                extensions: if is_fcgi_php {
                    HashSet::from([".php".into()])
                } else {
                    HashSet::new()
                },
                backend_server: backend_server?,
                environment: HashMap::new(),
                local_limit: None,
                pass: !is_fcgi_php,
                keepalive: false,
            });
        };

        if backend_server.is_none() {
            backend_server = cgi_children
                .get_value("backend")
                .and_then(|v| v.as_string_with_interpolations(ctx));
        }

        let mut local_limit = None;
        if let Some(limit_entries) = cgi_children.directives.get("limit") {
            if let Some(entry) = limit_entries.first() {
                if entry.args.len() == 1 {
                    if let Some(limit) = entry.args[0].as_number() {
                        local_limit = Some(limit as usize);
                    }
                }
            }
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

        let mut extensions = HashSet::new();
        if is_fcgi_php {
            extensions.insert(".php".into());
        } else if let Some(entries) = cgi_children.directives.get("extension") {
            for entry in entries {
                for arg in &entry.args {
                    if let Some(extension) = arg.as_str() {
                        extensions.insert(extension.to_lowercase());
                    }
                }
            }
        }

        let pass = if is_fcgi_php {
            false
        } else {
            cgi_children
                .directives
                .get("pass")
                .and_then(|e| e.first())
                .is_none_or(|e| e.get_flag())
        };
        let keepalive = cgi_children.get_flag("keepalive");

        Some(FcgiConfiguration {
            extensions,
            backend_server: backend_server?,
            environment,
            local_limit,
            pass,
            keepalive,
        })
    }

    pub fn from_http_ctx(ctx: &HttpContext) -> Vec<Self> {
        let mut result = Vec::new();
        for entry in ctx.configuration.get_entries("fcgi_php", true) {
            if let Some(config) = Self::from_config_entry(entry, true, ctx) {
                result.push(config);
            }
        }
        for entry in ctx.configuration.get_entries("fcgi", true) {
            if let Some(config) = Self::from_config_entry(entry, false, ctx) {
                result.push(config);
            }
        }
        result
    }
}
