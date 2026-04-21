use std::collections::{HashMap, HashSet};

use ferron_http::HttpContext;

/*
  cgi {
    extension ".php"
    interpreter ".php" php-cgi ...
    # or interprerter ".php" false
    environment "A" "B"
  }
*/

pub struct CgiConfiguration {
    pub additional_extensions: HashSet<String>,
    pub interpreters: HashMap<String, Option<Vec<String>>>,
    pub environment: HashMap<String, String>,
}

impl CgiConfiguration {
    pub fn from_http_ctx(ctx: &HttpContext) -> Option<Self> {
        let cgi_config = ctx.configuration.get_entry("cgi", true)?;
        if !cgi_config.get_flag() {
            return None;
        }
        let Some(cgi_children) = cgi_config.children.as_ref() else {
            return Some(CgiConfiguration {
                additional_extensions: HashSet::new(),
                interpreters: HashMap::new(),
                environment: HashMap::new(),
            });
        };

        let mut additional_extensions = HashSet::new();
        if let Some(entries) = cgi_children.directives.get("extension") {
            for entry in entries {
                for arg in &entry.args {
                    if let Some(extension) = arg.as_str() {
                        additional_extensions.insert(extension.to_lowercase());
                    }
                }
            }
        }

        let mut interpreters = HashMap::new();
        if let Some(entries) = cgi_children.directives.get("interpreter") {
            for entry in entries {
                let mut args_iter = entry.args.iter();
                if let Some(extension) = args_iter.next().and_then(|v| v.as_str()) {
                    let mut interpeter_args = Vec::new();
                    for arg in args_iter {
                        if arg.as_boolean() == Some(false) {
                            interpeter_args.clear();
                            break;
                        } else if let Some(arg) = arg.as_str() {
                            interpeter_args.push(arg.to_string());
                        }
                    }

                    interpreters.insert(
                        extension.to_string(),
                        if !interpeter_args.is_empty() {
                            Some(interpeter_args)
                        } else {
                            None
                        },
                    );
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

        Some(CgiConfiguration {
            additional_extensions,
            interpreters,
            environment,
        })
    }
}
