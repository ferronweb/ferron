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

// Import utility modules from "util" directory
#[path = "util"]
mod ferron_util {
  pub mod anti_xss;
  pub mod cgi_response;
  pub mod combine_config;
  pub mod copy_move;
  pub mod error_pages;
  pub mod fcgi_decoder;
  pub mod fcgi_encoder;
  pub mod fcgi_name_value_pair;
  pub mod fcgi_record;
  pub mod generate_directory_listing;
  pub mod ip_blocklist;
  pub mod ip_match;
  pub mod load_config;
  pub mod load_tls;
  pub mod match_hostname;
  pub mod match_location;
  pub mod no_server_verifier;
  pub mod non_standard_code_structs;
  pub mod read_to_end_move;
  pub mod sizify;
  pub mod sni;
  pub mod split_stream_by_map;
  pub mod ttl_cache;
  pub mod url_rewrite_structs;
  pub mod url_sanitizer;
  pub mod validate_config;
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
  pub mod cache;
  pub mod cgi;
  pub mod fauth;
  pub mod fcgi;
  pub mod fproxy;
  pub mod rproxy;
  pub mod scgi;
}

// Standard library imports
use std::sync::Arc;
use std::{error::Error, path::PathBuf};

// External crate imports
use clap::Parser;
use ferron_common::{ServerConfig, ServerConfigRoot, ServerModule};
use ferron_server::start_server;
use ferron_util::load_config::load_config;
use libloading::{library_filename, Library, Symbol};
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
        let lib = match module_name {
          "rproxy" | "fproxy" | "cache" | "cgi" | "scgi" | "fcgi" | "fauth" => None,
          _ => Some(
            match unsafe {
              Library::new(library_filename(format!(
                "ferron_mod_{}",
                module_name.replace("/", "_")
              )))
            } {
              Ok(lib) => lib,
              Err(err) => {
                module_error = Some(anyhow::anyhow!(
                  "Cannot load module \"{}\": {}",
                  module_name,
                  err
                ));
                break;
              }
            },
          ),
        };

        module_libs.push((lib, String::from(module_name)));
      }
    }
  }

  let mut external_modules = Vec::new();
  let mut module_config_validation_functions = Vec::new();
  let mut modules_optional_builtin = Vec::new();
  // Iterate over loaded module libraries and initialize them
  for (lib, module_name) in module_libs.iter() {
    if let Some(lib) = lib {
      // Retrieve the module initialization function
      let module_init: Symbol<
        fn(
          &ServerConfig,
        ) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>>,
      > = match unsafe { lib.get(b"server_module_init") } {
        Ok(module) => module,
        Err(err) => {
          module_error = Some(anyhow::anyhow!(
            "Cannot load module \"{}\": {}",
            module_name,
            err
          ));
          break;
        }
      };

      // Initialize the module
      external_modules.push(match module_init(&yaml_config) {
        Ok(module) => module,
        Err(err) => {
          module_error = Some(anyhow::anyhow!(
            "Cannot initialize module \"{}\": {}",
            module_name,
            err
          ));
          break;
        }
      });

      // Retrieve the module configuration validation function
      let module_validate_config: Symbol<
        fn(&ServerConfigRoot, bool, bool) -> Result<(), Box<dyn Error + Send + Sync>>,
      > = match unsafe { lib.get(b"server_module_validate_config") } {
        Ok(module) => module,
        Err(err) => {
          module_error = Some(anyhow::anyhow!(
            "Cannot load module \"{}\": {}",
            module_name,
            err
          ));
          break;
        }
      };
      module_config_validation_functions.push(module_validate_config);
    } else {
      match module_name as &str {
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
        _ => {
          module_error = Some(anyhow::anyhow!(
            "The optional built-in module \"{}\" doesn't exist",
            module_name
          ));
          break;
        }
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
    module_config_validation_functions,
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
        eprintln!("FATAL ERROR: {}", err);
        std::process::exit(1);
      }
    }
  }
}
