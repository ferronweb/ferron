use std::{
  collections::{HashMap, HashSet},
  error::Error,
  fs,
  net::{IpAddr, SocketAddr},
  path::{Path, PathBuf},
  str::FromStr,
};

use ferron_common::observability::ObservabilityBackendChannels;
use glob::glob;

use crate::config::{
  parse_conditional_data, Conditional, ConditionalData, Conditions, ErrorHandlerStatus, ServerConfiguration,
  ServerConfigurationEntries, ServerConfigurationEntry, ServerConfigurationFilters, ServerConfigurationValue,
};

use super::ConfigurationAdapter;

fn kdlite_error_near(pos: usize, file_contents: &str) -> String {
  let part = file_contents
    .split_at_checked(pos)
    .map(|split| split.1.split_at_checked(50).map_or(split.1, |split2| split2.0))
    .and_then(|part| if part.is_empty() { None } else { Some(part) });
  part.map_or("<end or out of bounds>".to_string(), |p| {
    snailquote::escape(p).to_string()
  })
}

fn display_kdlite_error(err: &kdlite::stream::Error, file_contents: &str) -> String {
  match err {
    kdlite::stream::Error::ExpectedSpace(index) => {
      format!("Expected space near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::ExpectedCloseParen(index) => {
      format!("Expected `)` near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::ExpectedComment(index) => format!(
      "Expected single-line comment near {}",
      kdlite_error_near(*index, file_contents)
    ),
    kdlite::stream::Error::ExpectedNewline(index) => {
      format!("Expected newline near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::ExpectedString(index) => {
      format!("Expected string near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::ExpectedValue(index) => {
      format!("Expected value near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::UnexpectedCloseBracket(index) => {
      format!("Unexpected `}}` near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::UnexpectedNewline(index) => {
      format!("Unexpected newline near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::InvalidNumber(index) => {
      format!("Invalid number near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::BadKeyword(index) => {
      format!("Invalid keyword name near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::BadIdentifier(index) => format!(
      "Invalid identifier name near {}",
      kdlite_error_near(*index, file_contents)
    ),
    kdlite::stream::Error::BadEscape(index) => format!(
      "Invalid escape sequence near {}",
      kdlite_error_near(*index, file_contents)
    ),
    kdlite::stream::Error::BadIndent(index) => {
      format!("Invalid indentation near {}", kdlite_error_near(*index, file_contents))
    }
    kdlite::stream::Error::MultipleChildren(index) => format!(
      "Multiple children for one KDL node near {}",
      kdlite_error_near(*index, file_contents)
    ),
    kdlite::stream::Error::UnexpectedEof => "Unexpected end of file".to_string(),
    kdlite::stream::Error::BannedChar(ch, index) => format!(
      "Invalid character `{}` near {}",
      ch.escape_default(),
      kdlite_error_near(*index, file_contents)
    ),
    _ => "Unknown error".to_string(),
  }
}

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
  let kdl_document = match kdlite::dom::Document::parse(&file_contents) {
    Ok(document) => document,
    Err(err) => Err(anyhow::anyhow!(
      "Failed to parse the server configuration file: {}",
      display_kdlite_error(&err, &file_contents)
    ))?,
  };

  // Loaded configuration vector
  let mut configurations = Vec::new();

  // Loaded conditions
  let mut loaded_conditions: HashMap<String, Vec<ConditionalData>> = HashMap::new();

  // KDL configuration snippets
  let mut snippets: HashMap<String, kdlite::dom::Document> = HashMap::new();

  // Iterate over KDL nodes
  for kdl_node in &kdl_document.nodes {
    let global_name = kdl_node.name();
    let children = &kdl_node.children;
    if global_name == "snippet" {
      if let Some(snippet_name) = kdl_node.entry(0).and_then(|v| match &v.value {
        kdlite::dom::Value::String(v) => Some(&**v),
        _ => None,
      }) {
        if let Some(children) = children {
          snippets.insert(snippet_name.to_string(), children.to_owned());
        } else {
          Err(anyhow::anyhow!("Snippet \"{snippet_name}\" is missing children"))?
        }
      } else {
        Err(anyhow::anyhow!("Invalid or missing snippet name"))?
      }
    } else if let Some(children) = children {
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
            let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

            Err(anyhow::anyhow!("Invalid host specifier at \"{}\"", canonical_path))?
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
            canonical_pathbuf: &PathBuf,
            host_filter: &(Option<String>, Option<IpAddr>, Option<u16>, bool),
            configurations: &mut Vec<ServerConfiguration>,
            configuration_entries: &mut HashMap<String, ServerConfigurationEntries>,
            kdl_node: &kdlite::dom::Node,
            conditions: &mut Option<&mut Conditions>,
            is_error_config: bool,
            loaded_conditions: &mut HashMap<String, Vec<ConditionalData>>,
            snippets: &HashMap<String, kdlite::dom::Document>,
          ) -> Result<(), Box<dyn Error + Send + Sync>> {
            let (hostname, ip, port, is_host) = host_filter;
            let kdl_node_name = kdl_node.name();
            let children = &kdl_node.children;
            if kdl_node_name == "use" {
              if let Some(snippet_name) = kdl_node.entry(0).and_then(|e| match &e.value {
                kdlite::dom::Value::String(s) => Some(&**s),
                _ => None,
              }) {
                if let Some(snippet) = snippets.get(snippet_name) {
                  for kdl_node in &snippet.nodes {
                    kdl_iterate_fn(
                      canonical_pathbuf,
                      host_filter,
                      configurations,
                      configuration_entries,
                      kdl_node,
                      conditions,
                      is_error_config,
                      loaded_conditions,
                      snippets,
                    )?;
                  }
                } else {
                  Err(anyhow::anyhow!(
                    "Snippet not defined: {snippet_name}. You might need to define it before using it"
                  ))?;
                }
              } else {
                Err(anyhow::anyhow!("Invalid `use` statement"))?;
              }
            } else if kdl_node_name == "location" {
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
                    let mut loaded_conditions = loaded_conditions.clone();
                    for kdl_node in &children.nodes {
                      kdl_iterate_fn(
                        canonical_pathbuf,
                        host_filter,
                        configurations,
                        &mut configuration_entries,
                        kdl_node,
                        &mut Some(&mut conditions),
                        is_error_config,
                        &mut loaded_conditions,
                        snippets,
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
                    let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                    Err(anyhow::anyhow!("Invalid location path at \"{}\"", canonical_path))?
                  }
                } else {
                  let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                  Err(anyhow::anyhow!("Invalid location at \"{}\"", canonical_path))?
                }
              } else {
                let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                Err(anyhow::anyhow!(
                  "Locations should have children, but they don't at \"{}\"",
                  canonical_path
                ))?
              }
            } else if kdl_node_name == "condition" {
              if is_error_config {
                Err(anyhow::anyhow!("Conditions in error configurations aren't allowed"))?;
              }
              if let Some(children) = children {
                if let Some(condition_name) = kdl_node.entry(0) {
                  if let Some(condition_name_str) = match &condition_name.value {
                    kdlite::dom::Value::String(s) => Some(&**s),
                    _ => None,
                  } {
                    let mut conditions_data = Vec::new();

                    let mut nodes_stack = Vec::new();
                    nodes_stack.push(children.nodes.iter());

                    while let Some(kdl_node) = {
                      let mut last_iterator_item = None;
                      while last_iterator_item.is_none() && !nodes_stack.is_empty() {
                        last_iterator_item = nodes_stack.last_mut().and_then(|i| i.next());
                        if last_iterator_item.is_none() {
                          nodes_stack.pop();
                        }
                      }
                      last_iterator_item
                    } {
                      let name = kdl_node.name();
                      if name == "use" {
                        if let Some(snippet_name) = kdl_node.entry(0).and_then(|v| match &v.value {
                          kdlite::dom::Value::String(s) => Some(&**s),
                          _ => None,
                        }) {
                          if let Some(snippet) = snippets.get(snippet_name) {
                            nodes_stack.push(snippet.nodes.iter());
                            continue;
                          } else {
                            Err(anyhow::anyhow!(
                              "Snippet not defined: {snippet_name}. You might need to define it before using it"
                            ))?;
                          }
                        } else {
                          Err(anyhow::anyhow!("Invalid `use` statement"))?;
                        }
                      }
                      let value = kdl_node_to_configuration_entry(kdl_node);
                      conditions_data.push(match parse_conditional_data(name, value) {
                        Ok(d) => d,
                        Err(err) => Err(anyhow::anyhow!(
                          "Invalid or unsupported subcondition at \"{condition_name_str}\" condition: {err}"
                        ))?,
                      });
                    }

                    loaded_conditions.insert(condition_name_str.to_string(), conditions_data);
                  } else {
                    let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                    Err(anyhow::anyhow!("Invalid location path at \"{}\"", canonical_path))?
                  }
                } else {
                  let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                  Err(anyhow::anyhow!("Invalid location at \"{}\"", canonical_path))?
                }
              } else {
                let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                Err(anyhow::anyhow!(
                  "Locations should have children, but they don't at \"{}\"",
                  canonical_path
                ))?
              }
            } else if kdl_node_name == "if" {
              if is_error_config {
                Err(anyhow::anyhow!("Conditions in error configurations aren't allowed"))?;
              }
              let mut configuration_entries: HashMap<String, ServerConfigurationEntries> = HashMap::new();
              if let Some(children) = children {
                if let Some(condition_name) = kdl_node.entry(0) {
                  if let Some(condition_name_str) = match &condition_name.value {
                    kdlite::dom::Value::String(s) => Some(&**s),
                    _ => None,
                  } {
                    let mut new_conditions = if let Some(conditions) = conditions {
                      conditions.clone()
                    } else {
                      Conditions {
                        location_prefix: "/".to_string(),
                        conditionals: vec![],
                      }
                    };

                    if let Some(conditionals) = loaded_conditions.get(condition_name_str) {
                      new_conditions.conditionals.push(Conditional::If(conditionals.clone()));
                    } else {
                      Err(anyhow::anyhow!(
                        "Condition not defined: {condition_name_str}. You might need to define it before using it"
                      ))?;
                    }

                    let mut loaded_conditions = loaded_conditions.clone();
                    for kdl_node in &children.nodes {
                      kdl_iterate_fn(
                        canonical_pathbuf,
                        host_filter,
                        configurations,
                        &mut configuration_entries,
                        kdl_node,
                        &mut Some(&mut new_conditions),
                        is_error_config,
                        &mut loaded_conditions,
                        snippets,
                      )?;
                    }

                    configurations.push(ServerConfiguration {
                      entries: configuration_entries,
                      filters: ServerConfigurationFilters {
                        is_host: *is_host,
                        hostname: hostname.clone(),
                        ip: *ip,
                        port: *port,
                        condition: Some(new_conditions.to_owned()),
                        error_handler_status: None,
                      },
                      modules: vec![],
                      observability: ObservabilityBackendChannels::new(),
                    });
                  } else {
                    let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                    Err(anyhow::anyhow!("Invalid location path at \"{}\"", canonical_path))?
                  }
                } else {
                  let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                  Err(anyhow::anyhow!("Invalid location at \"{}\"", canonical_path))?
                }
              } else {
                let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                Err(anyhow::anyhow!(
                  "Locations should have children, but they don't at \"{}\"",
                  canonical_path
                ))?
              }
            } else if kdl_node_name == "if_not" {
              if is_error_config {
                Err(anyhow::anyhow!("Conditions in error configurations aren't allowed"))?;
              }
              let mut configuration_entries: HashMap<String, ServerConfigurationEntries> = HashMap::new();
              if let Some(children) = children {
                if let Some(condition_name) = kdl_node.entry(0) {
                  if let Some(condition_name_str) = match &condition_name.value {
                    kdlite::dom::Value::String(s) => Some(&**s),
                    _ => None,
                  } {
                    let mut new_conditions = if let Some(conditions) = conditions {
                      conditions.clone()
                    } else {
                      Conditions {
                        location_prefix: "/".to_string(),
                        conditionals: vec![],
                      }
                    };

                    if let Some(conditionals) = loaded_conditions.get(condition_name_str) {
                      new_conditions
                        .conditionals
                        .push(Conditional::IfNot(conditionals.clone()));
                    } else {
                      Err(anyhow::anyhow!(
                        "Condition not defined: {condition_name_str}. You might need to define it before using it"
                      ))?;
                    }

                    let mut loaded_conditions = loaded_conditions.clone();
                    for kdl_node in &children.nodes {
                      kdl_iterate_fn(
                        canonical_pathbuf,
                        host_filter,
                        configurations,
                        &mut configuration_entries,
                        kdl_node,
                        &mut Some(&mut new_conditions),
                        is_error_config,
                        &mut loaded_conditions,
                        snippets,
                      )?;
                    }

                    configurations.push(ServerConfiguration {
                      entries: configuration_entries,
                      filters: ServerConfigurationFilters {
                        is_host: *is_host,
                        hostname: hostname.clone(),
                        ip: *ip,
                        port: *port,
                        condition: Some(new_conditions.to_owned()),
                        error_handler_status: None,
                      },
                      modules: vec![],
                      observability: ObservabilityBackendChannels::new(),
                    });
                  } else {
                    let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                    Err(anyhow::anyhow!("Invalid location path at \"{}\"", canonical_path))?
                  }
                } else {
                  let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                  Err(anyhow::anyhow!("Invalid location at \"{}\"", canonical_path))?
                }
              } else {
                let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                Err(anyhow::anyhow!(
                  "Locations should have children, but they don't at \"{}\"",
                  canonical_path
                ))?
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
                    let mut loaded_conditions = loaded_conditions.clone();
                    for kdl_node in &children.nodes {
                      kdl_iterate_fn(
                        canonical_pathbuf,
                        host_filter,
                        configurations,
                        &mut configuration_entries,
                        kdl_node,
                        conditions,
                        true,
                        &mut loaded_conditions,
                        snippets,
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
                    let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

                    Err(anyhow::anyhow!(
                      "Invalid error handler status code at \"{}\"",
                      canonical_path
                    ))?
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
            Ok(())
          }
          kdl_iterate_fn(
            &canonical_pathbuf,
            &host_filter,
            &mut configurations,
            &mut configuration_entries,
            kdl_node,
            &mut None,
            false,
            &mut loaded_conditions,
            &snippets,
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
    } else if global_name == "include" {
      // Get the list of included files and include the configurations
      let mut include_files = Vec::new();
      for include_one in &kdl_node.entries {
        if include_one.key().is_some() {
          continue;
        }
        if let Some(include_glob) = match &include_one.value {
          kdlite::dom::Value::String(s) => Some(&**s),
          _ => None,
        } {
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
            include_files.push(fs::canonicalize(&file_globbed).unwrap_or_else(|_| file_globbed.clone()));
          }
        }
      }

      for included_file in include_files {
        configurations.extend(load_configuration_inner(included_file, loaded_paths)?);
      }
    } else {
      let canonical_path = canonical_pathbuf.to_string_lossy().into_owned();

      Err(anyhow::anyhow!("Invalid top-level directive at \"{}\"", canonical_path))?
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
  fn load_configuration(&self, path: &Path) -> Result<Vec<ServerConfiguration>, Box<dyn Error + Send + Sync>> {
    load_configuration_inner(path.to_path_buf(), &mut HashSet::new())
  }
}
