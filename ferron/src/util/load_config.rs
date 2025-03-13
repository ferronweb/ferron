use std::fs;
use std::path::PathBuf;
use std::{collections::HashSet, error::Error};

use glob::glob;
use yaml_rust2::{Yaml, YamlLoader};

pub fn load_config(path: PathBuf) -> Result<Yaml, Box<dyn Error + Send + Sync>> {
  load_config_inner(path, &mut HashSet::new())
}

fn load_config_inner(
  path: PathBuf,
  loaded_paths: &mut HashSet<PathBuf>,
) -> Result<Yaml, Box<dyn Error + Send + Sync>> {
  // Canonicalize the path
  let canonical_pathbuf = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());

  // Check if the path is duplicate. If it's not, add it to loaded paths.
  if loaded_paths.contains(&canonical_pathbuf) {
    let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

    Err(anyhow::anyhow!(
      "Detected the server configuration file include loop while attempting to load \"{}\"",
      canonical_path
    ))?
  } else {
    loaded_paths.insert(canonical_pathbuf.clone());
  }

  // Read the configuration file
  let file_contents = match fs::read_to_string(&path) {
    Ok(file) => file,
    Err(err) => {
      let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

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
  let mut yaml_config = yaml_configs[0].clone(); // Clone the first YAML document

  if yaml_config.is_hash() {
    // Get the list of included files
    let mut include_files = Vec::new();
    if let Some(include_yaml) = yaml_config["include"].as_vec() {
      for include_one_yaml in include_yaml.iter() {
        if let Some(include_glob) = include_one_yaml.as_str() {
          let files_globbed = match glob(include_glob) {
            Ok(files_globbed) => files_globbed,
            Err(err) => {
              let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

              Err(anyhow::anyhow!(
                "Failed to determine includes for the server configuration file at \"{}\": {}",
                canonical_path,
                err
              ))?
            }
          };

          for file_globbed_result in files_globbed {
            let file_globbed = match file_globbed_result {
              Ok(file_globbed) => file_globbed,
              Err(err) => {
                let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                Err(anyhow::anyhow!(
                  "Failed to determine includes for the server configuration file at \"{}\": {}",
                  canonical_path,
                  err
                ))?
              }
            };
            include_files
              .push(fs::canonicalize(&file_globbed).unwrap_or_else(|_| file_globbed.clone()));
          }
        }
      }
    }

    // Delete included configuration from YAML configuration
    if let Some(yaml_config_hash) = yaml_config.as_mut_hash() {
      yaml_config_hash.remove(&Yaml::String("include".to_string()));

      // Merge included configuration
      for included_file in include_files {
        let yaml_to_include = load_config_inner(included_file, loaded_paths)?;
        if let Some(yaml_to_include_hashmap) = yaml_to_include.as_hash() {
          for (key, value) in yaml_to_include_hashmap.iter() {
            if let Some(key) = key.as_str() {
              if key != "include" {
                match value {
                  Yaml::Array(host_array) => {
                    yaml_config_hash
                      .entry(Yaml::String(key.to_string()))
                      .and_modify(|global_val| {
                        if let Yaml::Array(global_array) = global_val {
                          global_array.extend(host_array.clone());
                        } else {
                          *global_val = Yaml::Array(host_array.clone());
                        }
                      })
                      .or_insert_with(|| Yaml::Array(host_array.clone()));
                  }
                  Yaml::Hash(host_hash) => {
                    yaml_config_hash
                      .entry(Yaml::String(key.to_string()))
                      .and_modify(|global_val| {
                        if let Yaml::Hash(global_hash) = global_val {
                          for (k, v) in host_hash {
                            global_hash.insert(k.clone(), v.clone());
                          }
                        } else {
                          *global_val = Yaml::Hash(host_hash.clone());
                        }
                      })
                      .or_insert_with(|| Yaml::Hash(host_hash.clone()));
                  }
                  _ => {
                    yaml_config_hash.insert(Yaml::String(key.to_string()), value.clone());
                  }
                }
              }
            }
          }
        }
      }
    }
  }

  // Return the server configuration
  Ok(yaml_config)
}
