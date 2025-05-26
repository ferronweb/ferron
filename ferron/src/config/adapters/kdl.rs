use std::{
  collections::{HashMap, HashSet},
  error::Error,
  fs,
  net::{IpAddr, SocketAddr},
  path::{Path, PathBuf},
  str::FromStr,
};

use glob::glob;
use kdl::{KdlDocument, KdlNode, KdlValue};

use crate::config::{
  ErrorHandlerStatus, ServerConfiguration, ServerConfigurationEntries, ServerConfigurationEntry,
  ServerConfigurationFilters, ServerConfigurationValue,
};

use super::ConfigurationAdapter;

fn kdl_node_to_configuration_entry(kdl_node: &KdlNode) -> ServerConfigurationEntry {
  let mut values = Vec::new();
  let mut props = HashMap::new();
  for kdl_entry in kdl_node.iter() {
    let value = match kdl_entry.value().to_owned() {
      KdlValue::String(value) => ServerConfigurationValue::String(value),
      KdlValue::Integer(value) => ServerConfigurationValue::Integer(value),
      KdlValue::Float(value) => ServerConfigurationValue::Float(value),
      KdlValue::Bool(value) => ServerConfigurationValue::Bool(value),
      KdlValue::Null => ServerConfigurationValue::Null,
    };
    if let Some(prop_name) = kdl_entry.name() {
      props.insert(prop_name.value().to_string(), value);
    } else {
      values.push(value);
    }
  }
  if values.is_empty() {
    // If KDL node doesn't have any arguments, add the "#true" KDL value
    values.push(ServerConfigurationValue::Bool(true));
  }
  ServerConfigurationEntry { values, props }
}

fn load_configuration_inner(
  path: PathBuf,
  loaded_paths: &mut HashSet<PathBuf>,
) -> Result<Vec<ServerConfiguration>, Box<dyn Error + Send + Sync>> {
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

  // Parse the configuration file contents
  let kdl_document: KdlDocument = match file_contents.parse() {
    Ok(document) => document,
    Err(err) => Err(anyhow::anyhow!(
      "Failed to parse the server configuration file: {}",
      err
    ))?,
  };

  // Loaded configuration vector
  let mut configurations = Vec::new();

  // Iterate over KDL nodes
  for kdl_node in kdl_document {
    let global_name = kdl_node.name().value();
    let children = kdl_node.children();
    if let Some(children) = children {
      let (hostname, ip, port) = if let Ok(socket_addr) = global_name.parse::<SocketAddr>() {
        (None, Some(socket_addr.ip()), Some(socket_addr.port()))
      } else if let Some((address, port_str)) = global_name.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
          if let Ok(ip_address) = address
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(address)
            .parse::<IpAddr>()
          {
            (None, Some(ip_address), Some(port))
          } else if address == "*" || address.is_empty() {
            (None, None, Some(port))
          } else {
            (Some(address.to_string()), None, Some(port))
          }
        } else if port_str == "*" {
          if let Ok(ip_address) = address
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(address)
            .parse::<IpAddr>()
          {
            (None, Some(ip_address), None)
          } else if address == "*" || address.is_empty() {
            (None, None, None)
          } else {
            (Some(address.to_string()), None, None)
          }
        } else {
          let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

          Err(anyhow::anyhow!(
            "Invalid host specifier at \"{}\"",
            canonical_path
          ))?
        }
      } else if let Ok(ip_address) = global_name
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(global_name)
        .parse::<IpAddr>()
      {
        (None, Some(ip_address), None)
      } else if global_name == "*" || global_name.is_empty() {
        (None, None, None)
      } else {
        (Some(global_name.to_string()), None, None)
      };

      let mut configuration_entries: HashMap<String, ServerConfigurationEntries> = HashMap::new();
      for kdl_node in children.nodes() {
        let kdl_node_name = kdl_node.name().value();
        let children = kdl_node.children();
        if kdl_node_name == "location" {
          let mut configuration_entries: HashMap<String, ServerConfigurationEntries> =
            HashMap::new();
          if let Some(children) = children {
            if let Some(location) = kdl_node.entry(0) {
              if let Some(location_str) = location.value().as_string() {
                for kdl_node in children.nodes() {
                  let kdl_node_name = kdl_node.name().value();
                  let children = kdl_node.children();
                  if kdl_node_name == "error_config" {
                    let mut configuration_entries: HashMap<String, ServerConfigurationEntries> =
                      HashMap::new();
                    if let Some(children) = children {
                      if let Some(error_status_code) = kdl_node.entry(0) {
                        if let Some(error_status_code) = error_status_code.value().as_integer() {
                          for kdl_node in children.nodes() {
                            let kdl_node_name = kdl_node.name().value();
                            let value = kdl_node_to_configuration_entry(kdl_node);
                            if let Some(entries) = configuration_entries.get_mut(kdl_node_name) {
                              entries.inner.push(value);
                            } else {
                              configuration_entries.insert(
                                kdl_node_name.to_string(),
                                ServerConfigurationEntries { inner: vec![value] },
                              );
                            }
                          }
                          configurations.push(ServerConfiguration {
                            entries: configuration_entries,
                            filters: ServerConfigurationFilters {
                              hostname: hostname.clone(),
                              ip,
                              port,
                              location_prefix: Some(location_str.to_string()),
                              error_handler_status: Some(ErrorHandlerStatus::Status(
                                error_status_code as u16,
                              )),
                            },
                            modules: vec![],
                          });
                        } else {
                          let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                          Err(anyhow::anyhow!(
                            "Invalid error handler status code at \"{}\"",
                            canonical_path
                          ))?
                        }
                      } else {
                        for kdl_node in children.nodes() {
                          let kdl_node_name = kdl_node.name().value();
                          let value = kdl_node_to_configuration_entry(kdl_node);
                          if let Some(entries) = configuration_entries.get_mut(kdl_node_name) {
                            entries.inner.push(value);
                          } else {
                            configuration_entries.insert(
                              kdl_node_name.to_string(),
                              ServerConfigurationEntries { inner: vec![value] },
                            );
                          }
                        }
                        configurations.push(ServerConfiguration {
                          entries: configuration_entries,
                          filters: ServerConfigurationFilters {
                            hostname: hostname.clone(),
                            ip,
                            port,
                            location_prefix: Some(location_str.to_string()),
                            error_handler_status: Some(ErrorHandlerStatus::Any),
                          },
                          modules: vec![],
                        });
                      }
                    } else {
                      let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                      Err(anyhow::anyhow!(
                        "Error handler blocks should have children, but they don't at \"{}\"",
                        canonical_path
                      ))?
                    }
                  } else {
                    let value = kdl_node_to_configuration_entry(kdl_node);
                    if let Some(entries) = configuration_entries.get_mut(kdl_node_name) {
                      entries.inner.push(value);
                    } else {
                      configuration_entries.insert(
                        kdl_node_name.to_string(),
                        ServerConfigurationEntries { inner: vec![value] },
                      );
                    }
                  }
                }
                if kdl_node
                  .entry("remove_base")
                  .and_then(|e| e.value().as_bool())
                  .unwrap_or(false)
                {
                  configuration_entries.insert(
                    "UNDOCUMENTED_REMOVE_PATH_PREFIX".to_string(),
                    ServerConfigurationEntries {
                      inner: vec![ServerConfigurationEntry {
                        values: vec![ServerConfigurationValue::String(location_str.to_string())],
                        props: HashMap::new(),
                      }],
                    },
                  );
                }
                configurations.push(ServerConfiguration {
                  entries: configuration_entries,
                  filters: ServerConfigurationFilters {
                    hostname: hostname.clone(),
                    ip,
                    port,
                    location_prefix: Some(location_str.to_string()),
                    error_handler_status: None,
                  },
                  modules: vec![],
                });
              } else {
                let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                Err(anyhow::anyhow!(
                  "Invalid location path at \"{}\"",
                  canonical_path
                ))?
              }
            } else {
              let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

              Err(anyhow::anyhow!(
                "Invalid location at \"{}\"",
                canonical_path
              ))?
            }
          } else {
            let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

            Err(anyhow::anyhow!(
              "Locations should have children, but they don't at \"{}\"",
              canonical_path
            ))?
          }
        } else if kdl_node_name == "error_config" {
          let mut configuration_entries: HashMap<String, ServerConfigurationEntries> =
            HashMap::new();
          if let Some(children) = children {
            if let Some(error_status_code) = kdl_node.entry(0) {
              if let Some(error_status_code) = error_status_code.value().as_integer() {
                for kdl_node in children.nodes() {
                  let kdl_node_name = kdl_node.name().value();
                  let value = kdl_node_to_configuration_entry(kdl_node);
                  if let Some(entries) = configuration_entries.get_mut(kdl_node_name) {
                    entries.inner.push(value);
                  } else {
                    configuration_entries.insert(
                      kdl_node_name.to_string(),
                      ServerConfigurationEntries { inner: vec![value] },
                    );
                  }
                }
                configurations.push(ServerConfiguration {
                  entries: configuration_entries,
                  filters: ServerConfigurationFilters {
                    hostname: hostname.clone(),
                    ip,
                    port,
                    location_prefix: None,
                    error_handler_status: Some(ErrorHandlerStatus::Status(
                      error_status_code as u16,
                    )),
                  },
                  modules: vec![],
                });
              } else {
                let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                Err(anyhow::anyhow!(
                  "Invalid error handler status code at \"{}\"",
                  canonical_path
                ))?
              }
            } else {
              for kdl_node in children.nodes() {
                let kdl_node_name = kdl_node.name().value();
                let value = kdl_node_to_configuration_entry(kdl_node);
                if let Some(entries) = configuration_entries.get_mut(kdl_node_name) {
                  entries.inner.push(value);
                } else {
                  configuration_entries.insert(
                    kdl_node_name.to_string(),
                    ServerConfigurationEntries { inner: vec![value] },
                  );
                }
              }
              configurations.push(ServerConfiguration {
                entries: configuration_entries,
                filters: ServerConfigurationFilters {
                  hostname: hostname.clone(),
                  ip,
                  port,
                  location_prefix: None,
                  error_handler_status: Some(ErrorHandlerStatus::Any),
                },
                modules: vec![],
              });
            }
          } else {
            let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

            Err(anyhow::anyhow!(
              "Error handler blocks should have children, but they don't at \"{}\"",
              canonical_path
            ))?
          }
        } else {
          let value = kdl_node_to_configuration_entry(kdl_node);
          if let Some(entries) = configuration_entries.get_mut(kdl_node_name) {
            entries.inner.push(value);
          } else {
            configuration_entries.insert(
              kdl_node_name.to_string(),
              ServerConfigurationEntries { inner: vec![value] },
            );
          }
        }
      }
      configurations.push(ServerConfiguration {
        entries: configuration_entries,
        filters: ServerConfigurationFilters {
          hostname,
          ip,
          port,
          location_prefix: None,
          error_handler_status: None,
        },
        modules: vec![],
      });
    } else if global_name == "include" {
      // Get the list of included files and include the configurations
      let mut include_files = Vec::new();
      for include_one in kdl_node.entries() {
        if include_one.name().is_some() {
          continue;
        }
        if let Some(include_glob) = include_one.value().as_string() {
          let include_glob_pathbuf = match PathBuf::from_str(include_glob) {
            Ok(pathbuf) => pathbuf,
            Err(err) => {
              let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

              Err(anyhow::anyhow!(
                "Failed to determine includes for the server configuration file at \"{}\": {}",
                canonical_path,
                err
              ))?
            }
          };
          let include_glob_pathbuf_canonicalized = if include_glob_pathbuf.is_absolute() {
            include_glob_pathbuf
          } else {
            let mut canonical_dirname = canonical_pathbuf.clone();
            canonical_dirname.pop();
            canonical_dirname.join(include_glob_pathbuf)
          };
          let files_globbed = match glob(&include_glob_pathbuf_canonicalized.to_string_lossy()) {
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

      for included_file in include_files {
        configurations.extend(load_configuration_inner(included_file, loaded_paths)?);
      }
    } else {
      let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

      Err(anyhow::anyhow!(
        "Invalid top-level directive at \"{}\"",
        canonical_path
      ))?
    }
  }

  Ok(configurations)
}

/// A KDL configuration adapter
pub struct KdlConfigurationAdapter;

impl KdlConfigurationAdapter {
  /// Creates a new configuration adapter
  pub fn new() -> Self {
    Self
  }
}

impl ConfigurationAdapter for KdlConfigurationAdapter {
  fn load_configuration(
    &self,
    path: &Path,
  ) -> Result<Vec<ServerConfiguration>, Box<dyn Error + Send + Sync>> {
    load_configuration_inner(path.to_path_buf(), &mut HashSet::new())
  }
}
