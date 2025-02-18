// Import server module from "server.rs"
#[path = "server.rs"]
mod project_karpacz_server;

// Import request handler module from "request_handler.rs"
#[path = "request_handler.rs"]
mod project_karpacz_request_handler;

// Import resources from "res" directory
#[path = "res"]
mod project_karpacz_res {
  pub mod server_software;
}

// Import utility modules from "util" directory
#[path = "util"]
mod project_karpacz_util {
  pub mod anti_xss;
  pub mod combine_config;
  pub mod error_pages;
  pub mod generate_directory_listing;
  pub mod ip_blocklist;
  pub mod ip_match;
  pub mod load_tls;
  pub mod match_hostname;
  pub mod non_standard_code_structs;
  pub mod sizify;
  pub mod sni;
  pub mod ttl_cache;
  pub mod url_rewrite_structs;
  pub mod url_sanitizer;
  pub mod validate_config;
}

// Import project modules from "modules" directory
#[path = "modules"]
mod project_karpacz_modules {
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
mod project_karpacz_optional_modules {
  pub mod fproxy;
  pub mod rproxy;
}

// Standard library imports
use std::error::Error;
use std::fs;
use std::sync::Arc;

// External crate imports
use clap::Parser;
use libloading::{library_filename, Library, Symbol};
use mimalloc::MiMalloc;
use project_karpacz_common::{ServerConfig, ServerConfigRoot, ServerModule};
use project_karpacz_server::start_server;
use yaml_rust2::YamlLoader;

// Set the global allocator to use mimalloc for performance optimization
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// Struct for command-line arguments
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
  /// The path to the server configuration file
  #[arg(short, long, default_value_t = String::from("./project-karpacz.yaml"))]
  config: String,
}

// Function to execute before starting the server
#[allow(clippy::type_complexity)]
fn before_starting_server(args: Args) -> Result<(), Box<dyn Error + Send + Sync>> {
  // Read the configuration file
  let file_contents = match fs::read_to_string(&args.config) {
    Ok(file) => file,
    Err(err) => {
      let canonical_path = fs::canonicalize(&args.config).map_or_else(
        |_| args.config.clone(),
        |path| {
          path
            .to_str()
            .map_or_else(|| args.config.clone(), |s| s.to_string())
        },
      );

      Err(anyhow::anyhow!(
        "Failed to read from the server configuration file at \"{}\": {}",
        canonical_path,
        err
      ))?
    }
  };

  // Load YAML configuration from the file contents
  let yaml_configs = match YamlLoader::load_from_str(&file_contents) {
    Ok(yaml_configs) => yaml_configs,
    Err(err) => Err(anyhow::anyhow!(
      "Failed to parse the server configuration file: {}",
      err
    ))?,
  };

  // Ensure the YAML file is not empty
  if yaml_configs.is_empty() {
    Err(anyhow::anyhow!(
      "No YAML documents detected in the server configuration file."
    ))?;
  }
  let yaml_config = yaml_configs[0].clone(); // Clone the first YAML document

  let mut module_error = None;
  let mut module_libs = Vec::new();

  // Load external modules defined in the configuration file
  if let Some(modules) = yaml_config["global"]["loadModules"].as_vec() {
    for module_name_yaml in modules.iter() {
      if let Some(module_name) = module_name_yaml.as_str() {
        let lib = match module_name {
          "rproxy" | "fproxy" => None,
          _ => Some(
            match unsafe {
              Library::new(library_filename(format!(
                "project_karpacz_mod_{}",
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
        fn(&ServerConfigRoot, bool) -> Result<(), Box<dyn Error + Send + Sync>>,
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
            match project_karpacz_optional_modules::rproxy::server_module_init(&yaml_config) {
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
            match project_karpacz_optional_modules::fproxy::server_module_init(&yaml_config) {
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
  match project_karpacz_modules::x_forwarded_for::server_module_init() {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match project_karpacz_modules::redirects::server_module_init() {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match project_karpacz_modules::blocklist::server_module_init(&yaml_config) {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match project_karpacz_modules::url_rewrite::server_module_init(&yaml_config) {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match project_karpacz_modules::non_standard_codes::server_module_init(&yaml_config) {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match project_karpacz_modules::redirect_trailing_slashes::server_module_init() {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  modules.append(&mut external_modules);
  match project_karpacz_modules::default_handler_checks::server_module_init() {
    Ok(module) => modules.push(module),
    Err(err) => {
      if module_error.is_none() {
        module_error = Some(anyhow::anyhow!("Cannot load a built-in module: {}", err));
      }
    }
  };
  match project_karpacz_modules::static_file_serving::server_module_init() {
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
  )?;

  Ok(())
}

// Entry point of the application
fn main() {
  let args = Args::parse(); // Parse command-line arguments
  match before_starting_server(args) {
    Ok(_) => (),
    Err(err) => {
      eprintln!("FATAL ERROR: {}", err);
      std::process::exit(1);
    }
  }
}
