// Import server module from "server.rs"
#[path = "server.rs"]
mod ferron_server;

// Import request handler module from "request_handler.rs"
#[path = "request_handler.rs"]
mod ferron_request_handler;

// Import resources from "res" directory
#[path = "res"]
mod ferron_res {
  pub mod server_software;
}

// Import common modules from "common" directory
#[path = "common/mod.rs"]
mod ferron_common;

// Import utility modules from "util" directory
#[path = "util"]
mod ferron_util {
  pub mod anti_xss;
  #[cfg(feature = "asgi")]
  pub mod asgi_messages;
  #[cfg(any(feature = "cgi", feature = "scgi", feature = "fcgi"))]
  pub mod cgi_response;
  pub mod combine_config;
  pub mod env_config;
  pub mod error_config;
  pub mod error_pages;
  #[cfg(feature = "fcgi")]
  pub mod fcgi_decoder;
  #[cfg(feature = "fcgi")]
  pub mod fcgi_encoder;
  #[cfg(feature = "fcgi")]
  pub mod fcgi_name_value_pair;
  #[cfg(feature = "fcgi")]
  pub mod fcgi_record;
  pub mod generate_directory_listing;
  pub mod ip_blocklist;
  pub mod ip_match;
  pub mod load_config;
  pub mod load_tls;
  pub mod match_hostname;
  pub mod match_location;
  #[cfg(any(feature = "rproxy", feature = "fauth"))]
  pub mod no_server_verifier;
  #[cfg(any(feature = "wsgi", feature = "wsgid", feature = "asgi"))]
  pub mod obtain_config_struct;
  pub mod obtain_config_struct_vec;
  #[cfg(all(unix, feature = "wsgid"))]
  pub mod preforked_process_pool;
  pub mod sizify;
  pub mod sni;
  #[cfg(feature = "fcgi")]
  pub mod split_stream_by_map;
  pub mod ttl_cache;
  pub mod url_sanitizer;
  pub mod validate_config;
  #[cfg(feature = "wsgi")]
  pub mod wsgi_error_stream;
  #[cfg(feature = "wsgi")]
  pub mod wsgi_input_stream;
  #[cfg(any(feature = "wsgi", feature = "wsgid"))]
  pub mod wsgi_load_application;
  #[cfg(feature = "wsgid")]
  pub mod wsgid_body_reader;
  #[cfg(feature = "wsgid")]
  pub mod wsgid_error_stream;
  #[cfg(feature = "wsgid")]
  pub mod wsgid_input_stream;
  #[cfg(feature = "wsgid")]
  pub mod wsgid_message_structs;
}

// Import project modules from "modules" directory
#[path = "modules"]
mod ferron_modules {
  pub mod blocklist;
  pub mod default_handler_checks;
  pub mod non_standard_codes;
  pub mod redirect_trailing_slashes;
  pub mod redirects;
  pub mod static_file_serving;
  pub mod url_rewrite;
  pub mod x_forwarded_for;
}

// Import optional project modules from "modules" directory
#[path = "optional_modules"]
mod ferron_optional_modules {
  #[cfg(feature = "asgi")]
  pub mod asgi;
  #[cfg(feature = "cache")]
  pub mod cache;
  #[cfg(feature = "cgi")]
  pub mod cgi;
  #[cfg(feature = "example")]
  pub mod example;
  #[cfg(feature = "fauth")]
  pub mod fauth;
  #[cfg(feature = "fcgi")]
  pub mod fcgi;
  #[cfg(feature = "fproxy")]
  pub mod fproxy;
  #[cfg(feature = "rproxy")]
  pub mod rproxy;
  #[cfg(feature = "scgi")]
  pub mod scgi;
  #[cfg(feature = "wsgi")]
  pub mod wsgi;
  #[cfg(feature = "wsgid")]
  pub mod wsgid;
}

// Standard library imports
use std::sync::Arc;
use std::{error::Error, path::PathBuf};

// External crate imports
use clap::Parser;
use ferron_server::start_server;
use ferron_util::load_config::load_config;
use mimalloc::MiMalloc;

// Set the global allocator to use mimalloc for performance optimization
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// Struct for command-line arguments
/// A fast, memory-safe web server written in Rust
#[derive(Parser, Debug)]
#[command(name = "Ferron")]
#[command(version, about, long_about = None)]
struct Args {
  /// The path to the server configuration file
  #[arg(short, long, default_value_t = String::from("./ferron.yaml"))]
  config: String,
}

// Function to execute before starting the server
#[allow(clippy::type_complexity)]
fn before_starting_server(
  args: &Args,
  first_start: bool,
) -> Result<bool, Box<dyn Error + Send + Sync>> {
  // Load the configuration
  let yaml_config = load_config(PathBuf::from(args.config.clone()))?;

  let mut module_error = None;
  let mut module_libs = Vec::new();

  // Load external modules defined in the configuration file
  if let Some(modules) = yaml_config["global"]["loadModules"].as_vec() {
    for module_name_yaml in modules.iter() {
      if let Some(module_name) = module_name_yaml.as_str() {
        module_libs.push(String::from(module_name));
      }
    }
  }

  let mut external_modules = Vec::new();
  #[allow(unused_mut)]
  let mut modules_optional_builtin = Vec::new();
  // Iterate over loaded module libraries and initialize them
  for module_name in module_libs.iter() {
    match module_name as &str {
      #[cfg(feature = "rproxy")]
      "rproxy" => {
        external_modules.push(
          match ferron_optional_modules::rproxy::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "fproxy")]
      "fproxy" => {
        external_modules.push(
          match ferron_optional_modules::fproxy::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "cache")]
      "cache" => {
        external_modules.push(
          match ferron_optional_modules::cache::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "cgi")]
      "cgi" => {
        external_modules.push(
          match ferron_optional_modules::cgi::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "scgi")]
      "scgi" => {
        external_modules.push(
          match ferron_optional_modules::scgi::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "fcgi")]
      "fcgi" => {
        external_modules.push(
          match ferron_optional_modules::fcgi::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "fauth")]
      "fauth" => {
        external_modules.push(
          match ferron_optional_modules::fauth::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "example")]
      "example" => {
        external_modules.push(
          match ferron_optional_modules::example::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "wsgi")]
      "wsgi" => {
        external_modules.push(
          match ferron_optional_modules::wsgi::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "wsgid")]
      "wsgid" => {
        external_modules.push(
          match ferron_optional_modules::wsgid::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      #[cfg(feature = "asgi")]
      "asgi" => {
        external_modules.push(
          match ferron_optional_modules::asgi::server_module_init(&yaml_config) {
            Ok(module) => module,
            Err(err) => {
              module_error = Some(anyhow::anyhow!(
                "Cannot initialize optional built-in module \"{}\": {}",
                module_name,
                err
              ));
              break;
            }
          },
        );

        modules_optional_builtin.push(module_name.clone());
      }
      _ => {
        module_error = Some(anyhow::anyhow!(
          "The optional built-in module \"{}\" doesn't exist",
          module_name
        ));
        break;
      }
    }
  }

  // Add modules (both built-in and loaded)
  let mut modules = Vec::new();
  match ferron_modules::x_forwarded_for::server_module_init() {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match ferron_modules::redirects::server_module_init() {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match ferron_modules::blocklist::server_module_init(&yaml_config) {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match ferron_modules::url_rewrite::server_module_init(&yaml_config) {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match ferron_modules::non_standard_codes::server_module_init(&yaml_config) {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match ferron_modules::redirect_trailing_slashes::server_module_init() {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  modules.append(&mut external_modules);
  match ferron_modules::default_handler_checks::server_module_init() {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match ferron_modules::static_file_serving::server_module_init() {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };

  // Start the server with configuration and loaded modules
  start_server(
    Arc::new(yaml_config),
    modules,
    module_error,
    modules_optional_builtin,
    first_start,
  )
}

// Entry point of the application
fn main() {
  let args = &Args::parse(); // Parse command-line arguments
  let mut first_start = true;
  loop {
    match before_starting_server(args, first_start) {
      Ok(false) => break,
      Ok(true) => {
        first_start = false;
        println!("Reloading the server configuration...");
      }
      Err(err) => {
        eprintln!("FATAL ERROR: {err}");
        std::process::exit(1);
      }
    }
  }
}
