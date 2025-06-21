use std::{error::Error, path::PathBuf};

use hashlink::LinkedHashMap;
use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use load_config::load_config;
use yaml_rust2::Yaml;

mod load_config;

/// Converts Ferron 1.x YAML configuration to Ferron 2.x KDL configuration
pub fn convert_yaml_to_kdl(
  input_path: PathBuf,
) -> Result<KdlDocument, Box<dyn Error + Send + Sync>> {
  let yaml_configuration = load_config(input_path)?;
  let mut kdl_configuration = KdlDocument::new();

  let kdl_configuration_nodes = kdl_configuration.nodes_mut();
  let (global_configuration, sni_configurations, load_server_modules, secure_port) =
    obtain_global_configuration(&yaml_configuration);
  kdl_configuration_nodes.push(global_configuration);
  for sni_configuration in sni_configurations {
    kdl_configuration_nodes.push(sni_configuration);
  }

  let (global_configuration, secure_global_configuration) =
    obtain_host_configuration(&yaml_configuration["global"], &load_server_modules);
  if !global_configuration.is_empty() {
    let mut kdl_global_configuration = KdlNode::new("*");
    kdl_global_configuration.set_children(global_configuration);
    kdl_global_configuration.set_format(kdl::KdlNodeFormat {
      leading: "// Global configuration".to_string(),
      ..Default::default()
    });
    kdl_configuration_nodes.push(kdl_global_configuration);
  }
  if let Some(secure_global_configuration) = secure_global_configuration {
    let mut kdl_global_configuration = KdlNode::new(format!("*:{}", secure_port));
    kdl_global_configuration.set_children(secure_global_configuration);
    kdl_global_configuration.set_format(kdl::KdlNodeFormat {
      leading: "// HTTPS global configuration".to_string(),
      ..Default::default()
    });
    kdl_configuration_nodes.push(kdl_global_configuration);
  }

  if let Some(hosts) = yaml_configuration["hosts"].as_vec() {
    for host in hosts {
      let hostname = if let Some(domain) = host["domain"].as_str() {
        if let Some((d_p1, d_p2)) = domain.rsplit_once(':') {
          if d_p2.parse::<u16>().is_ok() {
            Some(d_p1)
          } else {
            Some(domain)
          }
        } else {
          Some(domain)
        }
      } else {
        host["ip"].as_str()
      };
      if let Some(hostname) = hostname {
        let (host_configuration, secure_host_configuration) =
          obtain_host_configuration(host, &load_server_modules);
        if !host_configuration.is_empty() {
          let mut kdl_host_configuration = KdlNode::new(hostname);
          kdl_host_configuration.set_children(host_configuration);
          kdl_host_configuration.set_format(kdl::KdlNodeFormat {
            leading: format!("// Host configuration for \"{}\"", hostname),
            ..Default::default()
          });
          kdl_configuration_nodes.push(kdl_host_configuration);
        }
        if let Some(secure_host_configuration) = secure_host_configuration {
          let mut kdl_host_configuration = KdlNode::new(format!("{}:{}", hostname, secure_port));
          kdl_host_configuration.set_children(secure_host_configuration);
          kdl_host_configuration.set_format(kdl::KdlNodeFormat {
            leading: format!("// HTTPS host configuration for \"{}\"", hostname),
            ..Default::default()
          });
          kdl_configuration_nodes.push(kdl_host_configuration);
        }
      }
    }
  }

  kdl_configuration.autoformat();

  Ok(kdl_configuration)
}

pub fn obtain_host_configuration(
  yaml_subconfiguration: &Yaml,
  loaded_modules: &[String],
) -> (KdlDocument, Option<KdlDocument>) {
  let empty_hashmap = yaml_rust2::yaml::Hash::new();
  let yaml_properties = yaml_subconfiguration.as_hash().unwrap_or(&empty_hashmap);
  let mut kdl_config = KdlDocument::new();
  let mut kdl_secure_config = KdlDocument::new();

  let kdl_config_nodes = kdl_config.nodes_mut();
  let kdl_secure_config_nodes = kdl_secure_config.nodes_mut();

  let mut scgi_to = "tcp://localhost:4000/";
  let mut scgi_path = None;
  let mut fcgi_to = "tcp://localhost:4000/";
  let mut fcgi_path = None;
  let mut fcgi_script_extensions = &Vec::new();
  let mut wsgi_application_path = None;
  let mut wsgi_path = None;
  let mut wsgid_application_path = None;
  let mut wsgid_path = None;
  let mut asgi_application_path = None;
  let mut asgi_path = None;

  for (property, value) in yaml_properties {
    if let Some(property) = property.as_str() {
      match property {
        "locations" => {
          if let Some(locations) = value.as_vec() {
            for location in locations.iter().rev() {
              if let Some(location_path) = location["path"].as_str() {
                let (location_config, secure_location_config_option) =
                  obtain_host_configuration(location, loaded_modules);
                let mut kdl_location = KdlNode::new("location");
                kdl_location.push(KdlValue::String(location_path.to_string()));
                kdl_location.set_children(location_config);
                kdl_config_nodes.insert(0, kdl_location);
                if let Some(secure_location_config) = secure_location_config_option {
                  let mut kdl_location = KdlNode::new("location");
                  kdl_location.push(KdlValue::String(location_path.to_string()));
                  kdl_location.set_children(secure_location_config);
                  kdl_secure_config_nodes.insert(0, kdl_location);
                }
              }
            }
          }
        }
        "errorConfig" => {
          if let Some(error_configs) = value.as_vec() {
            for error_config in error_configs.iter().rev() {
              let (error_config_d, secure_error_config_d_option) =
                obtain_host_configuration(error_config, loaded_modules);
              let mut kdl_error_config = KdlNode::new("error_config");
              if let Some(status_code) = error_config["scode"].as_i64() {
                kdl_error_config.push(KdlValue::Integer(status_code as i128));
              }
              kdl_error_config.set_children(error_config_d);
              kdl_config_nodes.insert(0, kdl_error_config);
              if let Some(secure_error_config_d) = secure_error_config_d_option {
                let mut kdl_error_config = KdlNode::new("error_config");
                if let Some(status_code) = error_config["scode"].as_i64() {
                  kdl_error_config.push(KdlValue::Integer(status_code as i128));
                }
                kdl_error_config.set_children(secure_error_config_d);
                kdl_config_nodes.insert(0, kdl_error_config);
              }
            }
          }
        }
        "serverAdministratorEmail" => {
          if let Some(value) = value.as_str() {
            let mut kdl_property = KdlNode::new("server_administrator_email");
            kdl_property.push(KdlValue::String(value.to_string()));
            kdl_config_nodes.push(kdl_property);
          }
        }
        "customHeaders" => {
          if let Some(value) = value.as_hash() {
            for (header_name, header_value) in value {
              if let Some(header_name) = header_name.as_str() {
                if let Some(header_value) = header_value.as_str() {
                  let mut kdl_property = KdlNode::new("header");
                  kdl_property.push(KdlValue::String(header_name.to_string()));
                  kdl_property.push(KdlValue::String(header_value.to_string()));
                  kdl_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "disableToHTTPSRedirect" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("no_redirect_to_https");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_config_nodes.push(kdl_property);
          }
        }
        "wwwredirect" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("wwwredirect");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_config_nodes.push(kdl_property);
          }
        }
        "enableIPSpoofing" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("trust_x_forwarded_for");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_config_nodes.push(kdl_property);
          }
        }
        "allowDoubleSlashes" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("allow_double_slashes");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_config_nodes.push(kdl_property);
          }
        }
        "rewriteMap" => {
          if let Some(value) = value.as_vec() {
            for value in value {
              if let Some(regex) = value["regex"].as_str() {
                if let Some(replacement) = value["replacement"].as_str() {
                  let mut kdl_property = KdlNode::new("rewrite");
                  kdl_property.push(KdlValue::String(regex.to_string()));
                  kdl_property.push(KdlValue::String(replacement.to_string()));
                  if let Some(value) = value["isNotFile"].as_bool() {
                    kdl_property.push(KdlEntry::new_prop("file", KdlValue::Bool(!value)));
                  }
                  if let Some(value) = value["isNotDirectory"].as_bool() {
                    kdl_property.push(KdlEntry::new_prop("directory", KdlValue::Bool(!value)));
                  }
                  if let Some(value) = value["allowDoubleSlashes"].as_bool() {
                    kdl_property.push(KdlEntry::new_prop(
                      "allow_double_slashes",
                      KdlValue::Bool(value),
                    ));
                  }
                  if let Some(value) = value["last"].as_bool() {
                    kdl_property.push(KdlEntry::new_prop("last", KdlValue::Bool(value)));
                  }
                  kdl_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "enableRewriteLogging" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("rewrite_log");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_config_nodes.push(kdl_property);
          }
        }
        "wwwroot" => {
          if let Some(value) = value.as_str() {
            let mut kdl_property = KdlNode::new("root");
            kdl_property.push(KdlValue::String(value.to_string()));
            kdl_config_nodes.push(kdl_property);
          }
        }
        "disableTrailingSlashRedirects" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("no_trailing_redirect");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_config_nodes.push(kdl_property);
          }
        }
        "users" => {
          if let Some(value) = value.as_vec() {
            for value in value {
              if let Some(user) = value["name"].as_str() {
                if let Some(pass) = value["pass"].as_str() {
                  let mut kdl_property = KdlNode::new("user");
                  kdl_property.push(KdlValue::String(user.to_string()));
                  kdl_property.push(KdlValue::String(pass.to_string()));
                  kdl_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "nonStandardCodes" => {
          if let Some(value) = value.as_vec() {
            for value in value {
              if let Some(scode) = value["scode"].as_i64() {
                let mut kdl_property = KdlNode::new("status");
                kdl_property.push(KdlValue::Integer(scode as i128));
                if let Some(value) = value["url"].as_str() {
                  kdl_property.push(KdlEntry::new_prop(
                    "url",
                    KdlValue::String(value.to_string()),
                  ));
                }
                if let Some(value) = value["regex"].as_str() {
                  kdl_property.push(KdlEntry::new_prop(
                    "regex",
                    KdlValue::String(value.to_string()),
                  ));
                }
                if let Some(value) = value["location"].as_str() {
                  kdl_property.push(KdlEntry::new_prop(
                    "location",
                    KdlValue::String(value.to_string()),
                  ));
                }
                if let Some(value) = value["realm"].as_str() {
                  kdl_property.push(KdlEntry::new_prop(
                    "realm",
                    KdlValue::String(value.to_string()),
                  ));
                }
                if let Some(value) = value["disableBruteProtection"].as_bool() {
                  kdl_property.push(KdlEntry::new_prop(
                    "brute_protection",
                    KdlValue::Bool(!value),
                  ));
                }
                if let Some(value) = value["userList"].as_vec() {
                  kdl_property.push(KdlEntry::new_prop(
                    "users",
                    KdlValue::String(
                      value
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                    ),
                  ));
                }
                if let Some(value) = value["users"].as_vec() {
                  kdl_property.push(KdlEntry::new_prop(
                    "allowed",
                    KdlValue::String(
                      value
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                    ),
                  ));
                }
                kdl_config_nodes.push(kdl_property);
              }
            }
          }
        }
        "errorPages" => {
          if let Some(value) = value.as_vec() {
            for value in value {
              if let Some(scode) = value["scode"].as_i64() {
                if let Some(path) = value["path"].as_str() {
                  let mut kdl_property = KdlNode::new("error_page");
                  kdl_property.push(KdlValue::Integer(scode as i128));
                  kdl_property.push(KdlValue::String(path.to_string()));
                  kdl_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "enableETag" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("etag");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_config_nodes.push(kdl_property);
          }
        }
        "enableCompression" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("compressed");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_config_nodes.push(kdl_property);
          }
        }
        "enableDirectoryListing" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("directory_listing");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_config_nodes.push(kdl_property);
          }
        }
        "proxyTo" => {
          if loaded_modules.contains(&"rproxy".to_string()) {
            if let Some(value) = value.as_str() {
              let mut kdl_property = KdlNode::new("proxy");
              kdl_property.push(KdlValue::String(value.to_string()));
              kdl_config_nodes.push(kdl_property);
            } else if let Some(value) = value.as_vec() {
              for value in value {
                if let Some(value) = value.as_str() {
                  let mut kdl_property = KdlNode::new("proxy");
                  kdl_property.push(KdlValue::String(value.to_string()));
                  kdl_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "secureProxyTo" => {
          if loaded_modules.contains(&"rproxy".to_string()) {
            if let Some(value) = value.as_str() {
              let mut kdl_property = KdlNode::new("proxy");
              kdl_property.push(KdlValue::String(value.to_string()));
              kdl_secure_config_nodes.push(kdl_property);
            } else if let Some(value) = value.as_vec() {
              for value in value {
                if let Some(value) = value.as_str() {
                  let mut kdl_property = KdlNode::new("proxy");
                  kdl_property.push(KdlValue::String(value.to_string()));
                  kdl_secure_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "cacheVaryHeaders" => {
          if loaded_modules.contains(&"cache".to_string()) {
            if let Some(value) = value.as_vec() {
              for value in value {
                if let Some(value) = value.as_str() {
                  let mut kdl_property = KdlNode::new("cache_vary");
                  kdl_property.push(KdlValue::String(value.to_string()));
                  kdl_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "cacheIgnoreHeaders" => {
          if loaded_modules.contains(&"cache".to_string()) {
            if let Some(value) = value.as_vec() {
              for value in value {
                if let Some(value) = value.as_str() {
                  let mut kdl_property = KdlNode::new("cache_ignore");
                  kdl_property.push(KdlValue::String(value.to_string()));
                  kdl_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "maximumCacheResponseSize" => {
          if loaded_modules.contains(&"cache".to_string()) {
            if let Some(value) = value.as_i64() {
              let mut kdl_property = KdlNode::new("cache_max_response_size");
              kdl_property.push(KdlValue::Integer(value as i128));
              kdl_config_nodes.push(kdl_property);
            } else if value.is_null() {
              let mut kdl_property = KdlNode::new("cache_max_response_size");
              kdl_property.push(KdlValue::Null);
              kdl_config_nodes.push(kdl_property);
            }
          }
        }
        "cgiScriptExtensions" => {
          if loaded_modules.contains(&"cgi".to_string()) {
            if let Some(value) = value.as_vec() {
              for value in value {
                if let Some(value) = value.as_str() {
                  let mut kdl_property = KdlNode::new("cgi_extension");
                  kdl_property.push(KdlValue::String(value.to_string()));
                  kdl_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "cgiScriptInterpreters" => {
          if loaded_modules.contains(&"cgi".to_string()) {
            if let Some(value) = value.as_hash() {
              for (extension, interpreter) in value {
                if let Some(extension) = extension.as_str() {
                  if let Some(interpreter) = interpreter.as_vec() {
                    let mut kdl_property = KdlNode::new("cgi_interpreter");
                    kdl_property.push(KdlValue::String(extension.to_string()));
                    for value in interpreter {
                      if let Some(value) = value.as_str() {
                        kdl_property.push(KdlValue::String(value.to_string()));
                      }
                    }
                    kdl_config_nodes.push(kdl_property);
                  } else if interpreter.is_null() {
                    let mut kdl_property = KdlNode::new("cgi_interpreter");
                    kdl_property.push(KdlValue::String(extension.to_string()));
                    kdl_property.push(KdlValue::Null);
                    kdl_config_nodes.push(kdl_property);
                  }
                }
              }
            }
          }
        }
        "scgiTo" => {
          if let Some(value) = value.as_str() {
            scgi_to = value;
          }
        }
        "scgiPath" => {
          if let Some(value) = value.as_str() {
            scgi_path = Some(value);
          }
        }
        "fcgiScriptExtensions" => {
          if let Some(value) = value.as_vec() {
            fcgi_script_extensions = value;
          }
        }
        "fcgiTo" => {
          if let Some(value) = value.as_str() {
            fcgi_to = value;
          }
        }
        "fcgiPath" => {
          if let Some(value) = value.as_str() {
            fcgi_path = Some(value);
          }
        }
        "authTo" => {
          if loaded_modules.contains(&"fauth".to_string()) {
            if let Some(value) = value.as_str() {
              let mut kdl_property = KdlNode::new("auth_to");
              kdl_property.push(KdlValue::String(value.to_string()));
              kdl_config_nodes.push(kdl_property);
            }
          }
        }
        "forwardedAuthCopyHeaders" => {
          if loaded_modules.contains(&"fauth".to_string()) {
            if let Some(value) = value.as_vec() {
              for value in value {
                if let Some(value) = value.as_str() {
                  let mut kdl_property = KdlNode::new("auth_to_copy");
                  kdl_property.push(KdlValue::String(value.to_string()));
                  kdl_config_nodes.push(kdl_property);
                }
              }
            }
          }
        }
        "enableLoadBalancerHealthCheck" => {
          if loaded_modules.contains(&"rproxy".to_string()) {
            if let Some(value) = value.as_bool() {
              let mut kdl_property = KdlNode::new("lb_health_check");
              if !value {
                kdl_property.push(KdlValue::Bool(false));
              }
              kdl_config_nodes.push(kdl_property);
            }
          }
        }
        "loadBalancerHealthCheckMaximumFails" => {
          if loaded_modules.contains(&"rproxy".to_string()) {
            if let Some(value) = value.as_i64() {
              let mut kdl_property = KdlNode::new("lb_health_check_max_fails");
              kdl_property.push(KdlValue::Integer(value as i128));
              kdl_config_nodes.push(kdl_property);
            }
          }
        }
        "disableProxyCertificateVerification" => {
          if loaded_modules.contains(&"rproxy".to_string()) {
            if let Some(value) = value.as_bool() {
              let mut kdl_property = KdlNode::new("proxy_no_verification");
              if !value {
                kdl_property.push(KdlValue::Bool(false));
              }
              kdl_config_nodes.push(kdl_property);
            }
          }
        }
        "wsgiApplicationPath" => {
          if let Some(value) = value.as_str() {
            wsgi_application_path = Some(value);
          }
        }
        "wsgiPath" => {
          if let Some(value) = value.as_str() {
            wsgi_path = Some(value);
          }
        }
        "wsgidApplicationPath" => {
          if let Some(value) = value.as_str() {
            wsgid_application_path = Some(value);
          }
        }
        "wsgidPath" => {
          if let Some(value) = value.as_str() {
            wsgid_path = Some(value);
          }
        }
        "asgiApplicationPath" => {
          if let Some(value) = value.as_str() {
            asgi_application_path = Some(value);
          }
        }
        "asgiPath" => {
          if let Some(value) = value.as_str() {
            asgi_path = Some(value);
          }
        }
        "proxyInterceptErrors" => {
          if loaded_modules.contains(&"rproxy".to_string()) {
            if let Some(value) = value.as_bool() {
              let mut kdl_property = KdlNode::new("proxy_intercept_errors");
              if !value {
                kdl_property.push(KdlValue::Bool(false));
              }
              kdl_config_nodes.push(kdl_property);
            }
          }
        }
        _ => (),
      }
    }
  }

  if loaded_modules.contains(&"scgi".to_string()) {
    let mut kdl_scgi = KdlNode::new("scgi");
    kdl_scgi.push(KdlValue::String(scgi_to.to_string()));

    if let Some(scgi_path) = scgi_path {
      let mut kdl_location = KdlNode::new("location");
      kdl_location.push(KdlValue::String(scgi_path.to_string()));
      kdl_location.push(KdlEntry::new_prop("remove_base", KdlValue::Bool(true)));
      let mut location_config = KdlDocument::new();
      let location_config_nodes = location_config.nodes_mut();
      location_config_nodes.push(kdl_scgi);
      kdl_location.set_children(location_config);
      kdl_config_nodes.insert(0, kdl_location);
    } else {
      kdl_config_nodes.push(kdl_scgi);
    }
  }

  if loaded_modules.contains(&"fcgi".to_string()) {
    let mut kdl_fcgi = KdlNode::new("fcgi");
    kdl_fcgi.push(KdlValue::String(fcgi_to.to_string()));
    let mut kdl_fcgi_extensions = Vec::new();
    for value in fcgi_script_extensions {
      if let Some(value) = value.as_str() {
        let mut kdl_property = KdlNode::new("fcgi_extension");
        kdl_property.push(KdlValue::String(value.to_string()));
        kdl_fcgi_extensions.push(kdl_property);
      }
    }

    if let Some(fcgi_path) = fcgi_path {
      let mut kdl_location = KdlNode::new("location");
      kdl_location.push(KdlValue::String(fcgi_path.to_string()));
      kdl_location.push(KdlEntry::new_prop("remove_base", KdlValue::Bool(true)));
      let mut location_config = KdlDocument::new();
      let location_config_nodes = location_config.nodes_mut();
      location_config_nodes.push(kdl_fcgi);
      for kdl_fcgi_extension in kdl_fcgi_extensions {
        kdl_config_nodes.push(kdl_fcgi_extension);
      }
      kdl_location.set_children(location_config);
      kdl_config_nodes.insert(0, kdl_location);
    } else {
      kdl_fcgi.push(KdlEntry::new_prop("pass", KdlValue::Bool(false)));
      kdl_config_nodes.push(kdl_fcgi);
      for kdl_fcgi_extension in kdl_fcgi_extensions {
        kdl_config_nodes.push(kdl_fcgi_extension);
      }
    }
  }

  if loaded_modules.contains(&"wsgi".to_string()) {
    if let Some(wsgi_application_path) = wsgi_application_path {
      let mut kdl_wsgi = KdlNode::new("wsgi");
      kdl_wsgi.push(KdlValue::String(wsgi_application_path.to_string()));

      if let Some(wsgi_path) = wsgi_path {
        let mut kdl_location = KdlNode::new("location");
        kdl_location.push(KdlValue::String(wsgi_path.to_string()));
        kdl_location.push(KdlEntry::new_prop("remove_base", KdlValue::Bool(true)));
        let mut location_config = KdlDocument::new();
        let location_config_nodes = location_config.nodes_mut();
        location_config_nodes.push(kdl_wsgi);
        kdl_location.set_children(location_config);
        kdl_config_nodes.insert(0, kdl_location);
      } else {
        kdl_config_nodes.push(kdl_wsgi);
      }
    }
  }

  if loaded_modules.contains(&"wsgid".to_string()) {
    if let Some(wsgid_application_path) = wsgid_application_path {
      let mut kdl_wsgid = KdlNode::new("wsgid");
      kdl_wsgid.push(KdlValue::String(wsgid_application_path.to_string()));

      if let Some(wsgid_path) = wsgid_path {
        let mut kdl_location = KdlNode::new("location");
        kdl_location.push(KdlValue::String(wsgid_path.to_string()));
        kdl_location.push(KdlEntry::new_prop("remove_base", KdlValue::Bool(true)));
        let mut location_config = KdlDocument::new();
        let location_config_nodes = location_config.nodes_mut();
        location_config_nodes.push(kdl_wsgid);
        kdl_location.set_children(location_config);
        kdl_config_nodes.insert(0, kdl_location);
      } else {
        kdl_config_nodes.push(kdl_wsgid);
      }
    }
  }

  if loaded_modules.contains(&"asgi".to_string()) {
    if let Some(asgi_application_path) = asgi_application_path {
      let mut kdl_asgi = KdlNode::new("asgi");
      kdl_asgi.push(KdlValue::String(asgi_application_path.to_string()));

      if let Some(asgi_path) = asgi_path {
        let mut kdl_location = KdlNode::new("location");
        kdl_location.push(KdlValue::String(asgi_path.to_string()));
        kdl_location.push(KdlEntry::new_prop("remove_base", KdlValue::Bool(true)));
        let mut location_config = KdlDocument::new();
        let location_config_nodes = location_config.nodes_mut();
        location_config_nodes.push(kdl_asgi);
        kdl_location.set_children(location_config);
        kdl_config_nodes.insert(0, kdl_location);
      } else {
        kdl_config_nodes.push(kdl_asgi);
      }
    }
  }

  (
    kdl_config,
    if kdl_secure_config.is_empty() {
      None
    } else {
      Some(kdl_secure_config)
    },
  )
}

pub fn obtain_global_configuration(
  yaml_configuration: &Yaml,
) -> (KdlNode, Vec<KdlNode>, Vec<String>, u16) {
  let empty_hashmap = yaml_rust2::yaml::Hash::new();
  let yaml_global_properties = yaml_configuration["global"]
    .as_hash()
    .unwrap_or(&empty_hashmap);
  let mut kdl_global_properties = KdlNode::new("*");
  let mut kdl_global_children_to_insert = KdlDocument::new();
  let kdl_global_children_nodes = kdl_global_children_to_insert.nodes_mut();
  let mut sni_configuration = Vec::new();
  let mut load_server_modules = Vec::new();

  let mut port = 80;
  let mut secure_port = 443;
  let mut secure = false;
  let mut enable_http2 = true;
  let mut enable_http3 = false;
  let mut cert = None;
  let mut key = None;
  let mut disable_non_encrypted_server = false;
  let mut environment_variables = LinkedHashMap::new();
  let mut automatic_tls = false;

  if let Some(value) = yaml_global_properties
    .get(&Yaml::String("loadModules".to_string()))
    .and_then(|v| v.as_vec())
  {
    for module_yaml in value {
      if let Some(module) = module_yaml.as_str() {
        match module {
          "cgi" => {
            let kdl_property = KdlNode::new("cgi");
            kdl_global_children_nodes.push(kdl_property);
          }
          "cache" => {
            let kdl_property = KdlNode::new("cache");
            kdl_global_children_nodes.push(kdl_property);
          }
          "example" => {
            let kdl_property = KdlNode::new("example_handler");
            kdl_global_children_nodes.push(kdl_property);
          }
          "fproxy" => {
            let kdl_property = KdlNode::new("forward_proxy");
            kdl_global_children_nodes.push(kdl_property);
          }
          _ => (),
        }
        load_server_modules.push(module.to_string());
      }
    }
  }

  for (property, value) in yaml_global_properties {
    if let Some(property) = property.as_str() {
      match property {
        "port" => {
          if let Some(port_obtained) = value.as_i64() {
            port = port_obtained;
          }
        }
        "sport" => {
          if let Some(secure_port_obtained) = value.as_i64() {
            secure_port = secure_port_obtained;
          }
        }
        "secure" => {
          if let Some(secure_obtained) = value.as_bool() {
            secure = secure_obtained;
          }
        }
        "http2Settings" => {
          if let Some(http2_settings) = value.as_hash() {
            for (http2_setting, http2_setting_value) in http2_settings {
              if let Some(http2_setting) = http2_setting.as_str() {
                match http2_setting {
                  "initialWindowSize" => {
                    if let Some(http2_setting_value) = http2_setting_value.as_i64() {
                      let mut kdl_property = KdlNode::new("h2_initial_window_size");
                      kdl_property.push(KdlValue::Integer(http2_setting_value as i128));
                      kdl_global_children_nodes.push(kdl_property);
                    }
                  }
                  "maxFrameSize" => {
                    if let Some(http2_setting_value) = http2_setting_value.as_i64() {
                      let mut kdl_property = KdlNode::new("h2_max_frame_size");
                      kdl_property.push(KdlValue::Integer(http2_setting_value as i128));
                      kdl_global_children_nodes.push(kdl_property);
                    }
                  }
                  "maxConcurrentStreams" => {
                    if let Some(http2_setting_value) = http2_setting_value.as_i64() {
                      let mut kdl_property = KdlNode::new("h2_max_concurrent_streams");
                      kdl_property.push(KdlValue::Integer(http2_setting_value as i128));
                      kdl_global_children_nodes.push(kdl_property);
                    }
                  }
                  "maxHeaderListSize" => {
                    if let Some(http2_setting_value) = http2_setting_value.as_i64() {
                      let mut kdl_property = KdlNode::new("h2_max_header_list_size");
                      kdl_property.push(KdlValue::Integer(http2_setting_value as i128));
                      kdl_global_children_nodes.push(kdl_property);
                    }
                  }
                  "enableConnectProtocol" => {
                    if let Some(http2_setting_value) = http2_setting_value.as_bool() {
                      let mut kdl_property = KdlNode::new("h2_enable_connect_protocol");
                      kdl_property.push(KdlValue::Bool(http2_setting_value));
                      kdl_global_children_nodes.push(kdl_property);
                    }
                  }
                  _ => (),
                }
              }
            }
          }
        }
        "logFilePath" => {
          if let Some(value) = value.as_str() {
            let mut kdl_property = KdlNode::new("log");
            kdl_property.push(KdlValue::String(value.to_string()));
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "errorLogFilePath" => {
          if let Some(value) = value.as_str() {
            let mut kdl_property = KdlNode::new("error_log");
            kdl_property.push(KdlValue::String(value.to_string()));
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "enableHTTP2" => {
          if let Some(enable_http2_obtained) = value.as_bool() {
            enable_http2 = enable_http2_obtained;
          }
        }
        "enableHTTP3" => {
          if let Some(enable_http3_obtained) = value.as_bool() {
            enable_http3 = enable_http3_obtained;
          }
        }
        "cert" => {
          if let Some(value) = value.as_str() {
            cert = Some(value);
          }
        }
        "key" => {
          if let Some(value) = value.as_str() {
            key = Some(value);
          }
        }
        "sni" => {
          if let Some(sni) = value.as_hash() {
            for (sni_hostname, sni_data) in sni {
              if let Some(sni_hostname) = sni_hostname.as_str() {
                if let Some(sni_cert) = sni_data["cert"].as_str() {
                  if let Some(sni_key) = sni_data["key"].as_str() {
                    let mut kdl_tls = KdlNode::new("tls");
                    kdl_tls.push(KdlValue::String(sni_cert.to_string()));
                    kdl_tls.push(KdlValue::String(sni_key.to_string()));

                    let mut kdl_auto_tls = KdlNode::new("auto_tls");
                    kdl_auto_tls.push(KdlValue::Bool(false));

                    let mut kdl_sni_configuration = KdlNode::new(sni_hostname);
                    let mut kdl_sni_children_to_insert = KdlDocument::new();
                    let kdl_sni_children_nodes = kdl_sni_children_to_insert.nodes_mut();
                    kdl_sni_children_nodes.push(kdl_auto_tls);
                    kdl_sni_children_nodes.push(kdl_tls);
                    kdl_sni_configuration.set_children(kdl_sni_children_to_insert);
                    kdl_sni_configuration.set_format(kdl::KdlNodeFormat {
                      leading: format!("// SNI configuration for \"{}\"", sni_hostname),
                      ..Default::default()
                    });
                    sni_configuration.push(kdl_sni_configuration);
                  }
                }
              }
            }
          }
        }
        "useClientCertificate" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("tls_client_certificate");
            kdl_property.push(KdlValue::Bool(value));
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "cipherSuite" => {
          if let Some(value) = value.as_vec() {
            for value in value {
              if let Some(value) = value.as_str() {
                let mut kdl_property = KdlNode::new("tls_cipher_suite");
                kdl_property.push(KdlValue::String(value.to_string()));
                kdl_global_children_nodes.push(kdl_property);
              }
            }
          }
        }
        "ecdhCurve" => {
          if let Some(value) = value.as_vec() {
            for value in value {
              if let Some(value) = value.as_str() {
                let mut kdl_property = KdlNode::new("tls_ecdh_curves");
                kdl_property.push(KdlValue::String(value.to_string()));
                kdl_global_children_nodes.push(kdl_property);
              }
            }
          }
        }
        "tlsMinVersion" => {
          if let Some(value) = value.as_str() {
            let mut kdl_property = KdlNode::new("tls_min_version");
            kdl_property.push(KdlValue::String(value.to_string()));
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "tlsMaxVersion" => {
          if let Some(value) = value.as_str() {
            let mut kdl_property = KdlNode::new("tls_max_version");
            kdl_property.push(KdlValue::String(value.to_string()));
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "disableNonEncryptedServer" => {
          if let Some(disable_non_encrypted_server_obtained) = value.as_bool() {
            disable_non_encrypted_server = disable_non_encrypted_server_obtained;
          }
        }
        "blocklist" => {
          if let Some(value) = value.as_vec() {
            for value in value {
              if let Some(value) = value.as_str() {
                let mut kdl_property = KdlNode::new("block");
                kdl_property.push(KdlValue::String(value.to_string()));
                kdl_global_children_nodes.push(kdl_property);
              }
            }
          }
        }
        "enableOCSPStapling" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("ocsp_stapling");
            kdl_property.push(KdlValue::Bool(value));
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "environmentVariables" => {
          if let Some(env) = value.as_hash() {
            for (env_name, env_value) in env {
              if let Some(env_name) = env_name.as_str() {
                if let Some(env_value) = env_value.as_str() {
                  environment_variables.insert(env_name.to_string(), env_value.to_string());
                }
              }
            }
          }
        }
        "enableAutomaticTLS" => {
          if let Some(value) = value.as_bool() {
            automatic_tls = value;
          }
        }
        "automaticTLSContactEmail" => {
          if let Some(value) = value.as_str() {
            let mut kdl_property = KdlNode::new("auto_tls_contact");
            kdl_property.push(KdlValue::String(value.to_string()));
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "automaticTLSContactCacheDirectory" => {
          if let Some(value) = value.as_str() {
            let mut kdl_property = KdlNode::new("auto_tls_cache");
            kdl_property.push(KdlValue::String(value.to_string()));
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "automaticTLSLetsEncryptProduction" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("auto_tls_letsencrypt_production");
            if !value {
              kdl_property.push(KdlValue::Bool(false));
            }
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "useAutomaticTLSHTTPChallenge" => {
          if let Some(value) = value.as_bool() {
            let mut kdl_property = KdlNode::new("auto_tls_challenge");
            kdl_property.push(KdlValue::String(if value {
              "http-01".to_string()
            } else {
              "tls-alpn-01".to_string()
            }));
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "timeout" => {
          if let Some(value) = value.as_i64() {
            let mut kdl_property = KdlNode::new("timeout");
            kdl_property.push(KdlValue::Integer(value as i128));
            kdl_global_children_nodes.push(kdl_property);
          } else if value.is_null() {
            let mut kdl_property = KdlNode::new("timeout");
            kdl_property.push(KdlValue::Null);
            kdl_global_children_nodes.push(kdl_property);
          }
        }
        "loadBalancerHealthCheckWindow" => {
          if load_server_modules.contains(&"rproxy".to_string()) {
            if let Some(value) = value.as_i64() {
              let mut kdl_property = KdlNode::new("lb_health_check_window");
              kdl_property.push(KdlValue::Integer(value as i128));
              kdl_global_children_nodes.push(kdl_property);
            }
          }
        }
        "maximumCacheEntries" => {
          if load_server_modules.contains(&"cache".to_string()) {
            if let Some(value) = value.as_i64() {
              let mut kdl_property = KdlNode::new("cache_max_entries");
              kdl_property.push(KdlValue::Integer(value as i128));
              kdl_global_children_nodes.push(kdl_property);
            } else if value.is_null() {
              let mut kdl_property = KdlNode::new("cache_max_entries");
              kdl_property.push(KdlValue::Null);
              kdl_global_children_nodes.push(kdl_property);
            }
          }
        }
        "wsgiClearModuleImportPath" => {
          if load_server_modules.contains(&"wsgi".to_string()) {
            if let Some(value) = value.as_bool() {
              let mut kdl_property = KdlNode::new("wsgi_clear_imports");
              kdl_property.push(KdlValue::Bool(value));
              kdl_global_children_nodes.push(kdl_property);
            }
          }
        }
        "asgiClearModuleImportPath" => {
          if load_server_modules.contains(&"asgi".to_string()) {
            if let Some(value) = value.as_bool() {
              let mut kdl_property = KdlNode::new("asgi_clear_imports");
              kdl_property.push(KdlValue::Bool(value));
              kdl_global_children_nodes.push(kdl_property);
            }
          }
        }
        _ => (),
      }
    }
  }

  for (env_name, env_value) in environment_variables {
    if load_server_modules.contains(&"cgi".to_string()) {
      let mut kdl_environment = KdlNode::new("cgi_environment");
      kdl_environment.push(KdlValue::String(env_name.to_string()));
      kdl_environment.push(KdlValue::String(env_value.to_string()));
      kdl_global_children_nodes.insert(0, kdl_environment);
    }
    if load_server_modules.contains(&"fcgi".to_string()) {
      let mut kdl_environment = KdlNode::new("fcgi_environment");
      kdl_environment.push(KdlValue::String(env_name.to_string()));
      kdl_environment.push(KdlValue::String(env_value.to_string()));
      kdl_global_children_nodes.insert(0, kdl_environment);
    }
    if load_server_modules.contains(&"scgi".to_string()) {
      let mut kdl_environment = KdlNode::new("scgi_environment");
      kdl_environment.push(KdlValue::String(env_name.to_string()));
      kdl_environment.push(KdlValue::String(env_value.to_string()));
      kdl_global_children_nodes.insert(0, kdl_environment);
    }
    if load_server_modules.contains(&"wsgi".to_string()) {
      let mut kdl_environment = KdlNode::new("wsgi_environment");
      kdl_environment.push(KdlValue::String(env_name.to_string()));
      kdl_environment.push(KdlValue::String(env_value.to_string()));
      kdl_global_children_nodes.insert(0, kdl_environment);
    }
    if load_server_modules.contains(&"wsgid".to_string()) {
      let mut kdl_environment = KdlNode::new("wsgid_environment");
      kdl_environment.push(KdlValue::String(env_name.to_string()));
      kdl_environment.push(KdlValue::String(env_value.to_string()));
      kdl_global_children_nodes.insert(0, kdl_environment);
    }
  }
  if let Some(cert) = cert {
    if let Some(key) = key {
      let mut kdl_tls = KdlNode::new("tls");
      kdl_tls.push(KdlValue::String(cert.to_string()));
      kdl_tls.push(KdlValue::String(key.to_string()));
      kdl_global_children_nodes.insert(0, kdl_tls);
    }
  }
  let mut kdl_protocols = KdlNode::new("protocols");
  kdl_protocols.push(KdlValue::String("h1".to_string()));
  if enable_http2 {
    kdl_protocols.push(KdlValue::String("h2".to_string()));
  }
  if enable_http3 {
    kdl_protocols.push(KdlValue::String("h3".to_string()));
  }
  kdl_global_children_nodes.insert(0, kdl_protocols);

  let mut kdl_auto_tls = KdlNode::new("auto_tls");
  if !automatic_tls {
    kdl_auto_tls.push(KdlValue::Bool(false));
  }
  kdl_global_children_nodes.insert(0, kdl_auto_tls);

  let mut kdl_default_https_port = KdlNode::new("default_https_port");
  kdl_default_https_port.push(if secure {
    KdlValue::Integer(secure_port as i128)
  } else {
    KdlValue::Null
  });
  kdl_global_children_nodes.insert(0, kdl_default_https_port);

  let mut kdl_default_http_port = KdlNode::new("default_http_port");
  kdl_default_http_port.push(if disable_non_encrypted_server {
    KdlValue::Null
  } else {
    KdlValue::Integer(port as i128)
  });
  kdl_global_children_nodes.insert(0, kdl_default_http_port);

  kdl_global_properties.set_children(kdl_global_children_to_insert);

  kdl_global_properties.set_format(kdl::KdlNodeFormat {
    leading: "// Global-only configuration".to_string(),
    ..Default::default()
  });
  (
    kdl_global_properties,
    sni_configuration,
    load_server_modules,
    secure_port as u16,
  )
}
