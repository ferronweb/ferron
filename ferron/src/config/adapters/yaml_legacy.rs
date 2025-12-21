use std::{
  collections::HashMap,
  error::Error,
  net::{IpAddr, SocketAddr},
  path::Path,
};

use ferron_common::observability::ObservabilityBackendChannels;
use ferron_yaml2kdl_core::convert_yaml_to_kdl;

use crate::config::{
  Conditions, ErrorHandlerStatus, ServerConfiguration, ServerConfigurationEntries, ServerConfigurationEntry,
  ServerConfigurationFilters, ServerConfigurationValue,
};

use super::ConfigurationAdapter;

fn kdl_node_to_configuration_entry(kdl_node: &kdlite::dom::Node) -> ServerConfigurationEntry {
  let mut values = Vec::new();
  let mut props = HashMap::new();
  for kdl_entry in &kdl_node.entries {
    let value = match &kdl_entry.value {
      kdlite::dom::Value::String(value) => ServerConfigurationValue::String(value.to_string()),
      kdlite::dom::Value::Integer(value) => ServerConfigurationValue::Integer(*value),
      kdlite::dom::Value::Float(value) => ServerConfigurationValue::Float(*value),
      kdlite::dom::Value::Bool(value) => ServerConfigurationValue::Bool(*value),
      kdlite::dom::Value::Null => ServerConfigurationValue::Null,
    };
    if let Some(prop_name) = kdl_entry.key() {
      props.insert(prop_name.to_string(), value);
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

/// A legacy YAML configuration adapter that utilizes `ferron-yaml2kdl-core` component
pub struct YamlLegacyConfigurationAdapter;

impl YamlLegacyConfigurationAdapter {
  /// Creates a new configuration adapter
  pub fn new() -> Self {
    Self
  }
}

impl ConfigurationAdapter for YamlLegacyConfigurationAdapter {
  fn load_configuration(&self, path: &Path) -> Result<Vec<ServerConfiguration>, Box<dyn Error + Send + Sync>> {
    // Read and parse the configuration file contents
    let kdl_document: kdlite::dom::Document = match convert_yaml_to_kdl(path.to_path_buf()) {
      Ok(document) => document,
      Err(err) => Err(anyhow::anyhow!(
        "Failed to read and parse the server configuration file: {}",
        err
      ))?,
    };

    // Loaded configuration vector
    let mut configurations = Vec::new();

    // Iterate over KDL nodes
    for kdl_node in &kdl_document.nodes {
      let global_name = kdl_node.name();
      let children = &kdl_node.children;
      if let Some(children) = children {
        for global_name in global_name.split(",") {
          let host_filter = if global_name == "globals" {
            (None, None, None, false)
          } else if let Ok(socket_addr) = global_name.parse::<SocketAddr>() {
            (None, Some(socket_addr.ip()), Some(socket_addr.port()), true)
          } else if let Some((address, port_str)) = global_name.rsplit_once(':') {
            if let Ok(port) = port_str.parse::<u16>() {
              if let Ok(ip_address) = address
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or(address)
                .parse::<IpAddr>()
              {
                (None, Some(ip_address), Some(port), true)
              } else if address == "*" || address.is_empty() {
                (None, None, Some(port), true)
              } else {
                (Some(address.to_string()), None, Some(port), true)
              }
            } else if port_str == "*" {
              if let Ok(ip_address) = address
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or(address)
                .parse::<IpAddr>()
              {
                (None, Some(ip_address), None, true)
              } else if address == "*" || address.is_empty() {
                (None, None, None, true)
              } else {
                (Some(address.to_string()), None, None, true)
              }
            } else {
              Err(anyhow::anyhow!("Invalid host specifier"))?
            }
          } else if let Ok(ip_address) = global_name
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(global_name)
            .parse::<IpAddr>()
          {
            (None, Some(ip_address), None, true)
          } else if global_name == "*" || global_name.is_empty() {
            (None, None, None, true)
          } else {
            (Some(global_name.to_string()), None, None, true)
          };

          let mut configuration_entries: HashMap<String, ServerConfigurationEntries> = HashMap::new();
          for kdl_node in &children.nodes {
            #[allow(clippy::too_many_arguments)]
            fn kdl_iterate_fn(
              host_filter: &(Option<String>, Option<IpAddr>, Option<u16>, bool),
              configurations: &mut Vec<ServerConfiguration>,
              configuration_entries: &mut HashMap<String, ServerConfigurationEntries>,
              kdl_node: &kdlite::dom::Node,
              conditions: &mut Option<&mut Conditions>,
              is_error_config: bool,
            ) -> Result<(), Box<dyn Error + Send + Sync>> {
              let (hostname, ip, port, is_host) = host_filter;
              let kdl_node_name = kdl_node.name();
              let children = &kdl_node.children;
              if kdl_node_name == "location" {
                if is_error_config {
                  Err(anyhow::anyhow!("Locations in error configurations aren't allowed"))?;
                } else if conditions.is_some() {
                  Err(anyhow::anyhow!(
                    "Nested locations and locations in conditions aren't allowed"
                  ))?;
                }
                let mut configuration_entries: HashMap<String, ServerConfigurationEntries> = HashMap::new();
                if let Some(children) = children {
                  if let Some(location) = kdl_node.entry(0) {
                    if let Some(location_str) = match &location.value {
                      kdlite::dom::Value::String(s) => Some(&**s),
                      _ => None,
                    } {
                      let mut conditions = Conditions {
                        location_prefix: location_str.to_string(),
                        conditionals: vec![],
                      };
                      for kdl_node in &children.nodes {
                        kdl_iterate_fn(
                          host_filter,
                          configurations,
                          &mut configuration_entries,
                          kdl_node,
                          &mut Some(&mut conditions),
                          is_error_config,
                        )?;
                      }
                      if kdl_node
                        .entry("remove_base")
                        .and_then(|e| match &e.value {
                          kdlite::dom::Value::Bool(b) => Some(*b),
                          _ => None,
                        })
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
                          is_host: *is_host,
                          hostname: hostname.clone(),
                          ip: *ip,
                          port: *port,
                          condition: Some(conditions),
                          error_handler_status: None,
                        },
                        modules: vec![],
                        observability: ObservabilityBackendChannels::new(),
                      });
                    } else {
                      Err(anyhow::anyhow!("Invalid location path"))?
                    }
                  } else {
                    Err(anyhow::anyhow!("Invalid location"))?
                  }
                } else {
                  Err(anyhow::anyhow!("Locations should have children, but they don't"))?
                }
              } else if kdl_node_name == "error_config" {
                if is_error_config {
                  Err(anyhow::anyhow!("Nested error configurations aren't allowed"))?;
                }
                let mut configuration_entries: HashMap<String, ServerConfigurationEntries> = HashMap::new();
                if let Some(children) = children {
                  if let Some(error_status_code) = kdl_node.entry(0) {
                    if let Some(error_status_code) = match &error_status_code.value {
                      kdlite::dom::Value::Integer(i) => Some(*i),
                      _ => None,
                    } {
                      for kdl_node in &children.nodes {
                        kdl_iterate_fn(
                          host_filter,
                          configurations,
                          &mut configuration_entries,
                          kdl_node,
                          conditions,
                          true,
                        )?;
                      }
                      configurations.push(ServerConfiguration {
                        entries: configuration_entries,
                        filters: ServerConfigurationFilters {
                          is_host: *is_host,
                          hostname: hostname.clone(),
                          ip: *ip,
                          port: *port,
                          condition: None,
                          error_handler_status: Some(ErrorHandlerStatus::Status(error_status_code as u16)),
                        },
                        modules: vec![],
                        observability: ObservabilityBackendChannels::new(),
                      });
                    } else {
                      Err(anyhow::anyhow!("Invalid error handler status code"))?
                    }
                  } else {
                    for kdl_node in &children.nodes {
                      let kdl_node_name = kdl_node.name();
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
                        is_host: *is_host,
                        hostname: hostname.clone(),
                        ip: *ip,
                        port: *port,
                        condition: None,
                        error_handler_status: Some(ErrorHandlerStatus::Any),
                      },
                      modules: vec![],
                      observability: ObservabilityBackendChannels::new(),
                    });
                  }
                } else {
                  Err(anyhow::anyhow!(
                    "Error handler blocks should have children, but they don't"
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
              Ok(())
            }
            kdl_iterate_fn(
              &host_filter,
              &mut configurations,
              &mut configuration_entries,
              kdl_node,
              &mut None,
              false,
            )?;
          }
          let (hostname, ip, port, is_host) = host_filter;
          configurations.push(ServerConfiguration {
            entries: configuration_entries,
            filters: ServerConfigurationFilters {
              is_host,
              hostname,
              ip,
              port,
              condition: None,
              error_handler_status: None,
            },
            modules: vec![],
            observability: ObservabilityBackendChannels::new(),
          });
        }
      } else {
        Err(anyhow::anyhow!("Invalid top-level directive"))?
      }
    }

    Ok(configurations)
  }
}
