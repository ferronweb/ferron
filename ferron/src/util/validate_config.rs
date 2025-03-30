use crate::ferron_common::ServerConfig;
use hyper::header::{HeaderName, HeaderValue};
use std::error::Error;
use std::net::IpAddr;
use std::str::FromStr;
use yaml_rust2::Yaml;

fn validate_ip(ip: &str) -> bool {
  let _: IpAddr = match ip.parse() {
    Ok(addr) => addr,
    Err(_) => return false,
  };
  true
}

// Internal configuration file validators
pub fn validate_config(
  config: ServerConfig,
  is_global: bool,
  is_location: bool,
  modules_optional_builtin: &[String],
) -> Result<(), Box<dyn Error + Send + Sync>> {
  let domain_badvalue = config["domain"].is_badvalue();
  let ip_badvalue = config["ip"].is_badvalue();

  if !domain_badvalue && config["domain"].as_str().is_none() {
    Err(anyhow::anyhow!("Invalid domain name"))?
  }

  if !ip_badvalue {
    match config["ip"].as_str() {
      Some(ip) => {
        if !validate_ip(ip) {
          Err(anyhow::anyhow!("Invalid IP address"))?;
        }
      }
      None => {
        Err(anyhow::anyhow!("Invalid IP address"))?;
      }
    }
  }

  if domain_badvalue && ip_badvalue && !is_global && !is_location {
    Err(anyhow::anyhow!(
      "A host must either have IP address or domain name specified"
    ))?;
  }

  if !config["path"].is_badvalue() {
    if !is_location {
      Err(anyhow::anyhow!(
        "Location path configuration is only allowed in location configuration"
      ))?;
    }
    if config["path"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid location path"))?;
    }
  }

  if !config["locations"].is_badvalue() && is_location {
    Err(anyhow::anyhow!("Nested locations are not allowed"))?;
  }

  if !config["loadModules"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Module configuration is not allowed in host configuration"
      ))?
    }
    if let Some(modules) = config["loadModules"].as_vec() {
      let modules_iter = modules.iter();
      for module_name_yaml in modules_iter {
        if module_name_yaml.as_str().is_none() {
          Err(anyhow::anyhow!("Invalid module name"))?
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid module configuration"))?
    }
  }

  if !config["port"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTP port configuration is not allowed in host configuration"
      ))?
    }
    if let Some(port) = config["port"].as_i64() {
      if !(0..=65535).contains(&port) {
        Err(anyhow::anyhow!("Invalid HTTP port"))?
      }
    } else if config["port"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid HTTP port"))?
    }
  }

  if !config["sport"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTPS port configuration is not allowed in host configuration"
      ))?
    }
    if let Some(port) = config["sport"].as_i64() {
      if !(0..=65535).contains(&port) {
        Err(anyhow::anyhow!("Invalid HTTPS port"))?
      }
    } else if config["sport"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid HTTPS port"))?
    }
  }

  if !config["secure"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTPS enabling configuration is not allowed in host configuration"
      ))?
    }
    if config["secure"].as_bool().is_none() {
      Err(anyhow::anyhow!("Invalid HTTPS enabling option value"))?
    }
  }

  if !config["enableHTTP2"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTP/2 enabling configuration is not allowed in host configuration"
      ))?
    }
    if config["enableHTTP2"].as_bool().is_none() {
      Err(anyhow::anyhow!("Invalid HTTP/2 enabling option value"))?
    }
  }

  if !config["logFilePath"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Log file configuration is not allowed in host configuration"
      ))?
    }
    if config["logFilePath"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid log file path"))?
    }
  }

  if !config["errorLogFilePath"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Error log file configuration is not allowed in host configuration"
      ))?
    }
    if config["errorLogFilePath"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid error log file path"))?
    }
  }

  if !config["cert"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "TLS certificate configuration is not allowed in host configuration"
      ))?
    }
    if config["cert"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid TLS certificate path"))?
    }
  }

  if !config["key"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Private key configuration is not allowed in host configuration"
      ))?
    }
    if config["key"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid private key path"))?
    }
  }

  if !config["sni"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "SNI configuration is not allowed in host configuration"
      ))?
    }
    if let Some(sni) = config["sni"].as_hash() {
      let sni_hostnames = sni.keys();
      for sni_hostname_unknown in sni_hostnames {
        if let Some(sni_hostname) = sni_hostname_unknown.as_str() {
          if sni[sni_hostname_unknown]["cert"].as_str().is_none() {
            Err(anyhow::anyhow!(
              "Invalid SNI TLS certificate path for \"{}\"",
              sni_hostname
            ))?
          }
          if sni[sni_hostname_unknown]["key"].as_str().is_none() {
            Err(anyhow::anyhow!(
              "Invalid SNI private key certificate path for \"{}\"",
              sni_hostname
            ))?
          }
        } else {
          Err(anyhow::anyhow!("Invalid SNI hostname"))?
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid SNI certificate list"))?
    }
  }

  if !config["http2Options"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTP/2 configuration is not allowed in host configuration"
      ))?
    }
    if config["http2Options"].as_hash().is_some() {
      if let Some(initial_window_size) = config["http2Options"]["initialWindowSize"].as_i64() {
        if !(0..=2_147_483_647).contains(&initial_window_size) {
          Err(anyhow::anyhow!("Invalid HTTP/2 initial window size"))?
        }
      }

      if let Some(max_frame_size) = config["http2Options"]["maxFrameSize"].as_i64() {
        if !(16_384..=16_777_215).contains(&max_frame_size) {
          Err(anyhow::anyhow!("Invalid HTTP/2 max frame size"))?
        }
      }

      if let Some(max_concurrent_streams) = config["http2Options"]["maxConcurrentStreams"].as_i64()
      {
        if max_concurrent_streams < 0 {
          Err(anyhow::anyhow!("Invalid HTTP/2 max concurrent streams"))?
        }
      }

      if let Some(max_header_list_size) = config["http2Options"]["maxHeaderListSize"].as_i64() {
        if max_header_list_size < 0 {
          Err(anyhow::anyhow!("Invalid HTTP/2 max header list size"))?
        }
      }

      if !config["http2Options"]["enableConnectProtocol"].is_badvalue()
        && config["http2Options"]["enableConnectProtocol"]
          .as_bool()
          .is_none()
      {
        Err(anyhow::anyhow!(
          "Invalid HTTP/2 enable connect protocol option"
        ))?
      }
    } else {
      Err(anyhow::anyhow!("Invalid HTTP/2 options"))?
    }
  }

  if !config["useClientCertificate"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Client certificate verfication enabling option is not allowed in host configuration"
      ))?
    }
    if config["useClientCertificate"].as_bool().is_none() {
      Err(anyhow::anyhow!(
        "Invalid client certificate verification enabling option value"
      ))?
    }
  }

  if !config["cipherSuite"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Cipher suite configuration is not allowed in host configuration"
      ))?
    }
    if let Some(cipher_suites) = config["cipherSuite"].as_vec() {
      let cipher_suites_iter = cipher_suites.iter();
      for cipher_suite_name_yaml in cipher_suites_iter {
        if cipher_suite_name_yaml.as_str().is_none() {
          Err(anyhow::anyhow!("Invalid cipher suite"))?
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid cipher suite configuration"))?
    }
  }

  if !config["ecdhCurve"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "ECDH curve configuration is not allowed in host configuration"
      ))?
    }
    if let Some(ecdh_curves) = config["ecdhCurve"].as_vec() {
      let ecdh_curves_iter = ecdh_curves.iter();
      for ecdh_curve_name_yaml in ecdh_curves_iter {
        if ecdh_curve_name_yaml.as_str().is_none() {
          Err(anyhow::anyhow!("Invalid ECDH curve"))?
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid ECDH curve configuration"))?
    }
  }

  if !config["tlsMinVersion"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Minimum TLS version is not allowed in host configuration"
      ))?
    }
    if config["tlsMinVersion"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid minimum TLS version"))?
    }
  }

  if !config["tlsMaxVersion"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Maximum TLS version is not allowed in host configuration"
      ))?
    }
    if config["tlsMaxVersion"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid maximum TLS version"))?
    }
  }

  if !config["enableOCSPStapling"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "OCSP stapling enabling option is not allowed in host configuration"
      ))?
    }
    if config["enableOCSPStapling"].as_bool().is_none() {
      Err(anyhow::anyhow!(
        "Invalid OCSP stapling enabling option value"
      ))?
    }
  }

  if !config["serverAdministratorEmail"].is_badvalue()
    && config["serverAdministratorEmail"].as_str().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid server administrator email address"
    ))?
  }

  if !config["enableIPSpoofing"].is_badvalue() && config["enableIPSpoofing"].as_bool().is_none() {
    Err(anyhow::anyhow!(
      "Invalid X-Forwarded-For enabling option value"
    ))?
  }

  if !config["disableNonEncryptedServer"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Non-encrypted server disabling option is not allowed in host configuration"
      ))?
    }
    if config["disableNonEncryptedServer"].as_bool().is_none() {
      Err(anyhow::anyhow!(
        "Invalid non-encrypted server disabling option value"
      ))?
    }
  }

  if !config["blocklist"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Block list configuration is not allowed in host configuration"
      ))?
    }
    if let Some(blocklist) = config["blocklist"].as_vec() {
      let blocklist_iter = blocklist.iter();
      for blocklist_entry_yaml in blocklist_iter {
        match blocklist_entry_yaml.as_str() {
          Some(blocklist_entry) => {
            if !validate_ip(blocklist_entry) {
              Err(anyhow::anyhow!("Invalid block list entry"))?
            }
          }
          None => Err(anyhow::anyhow!("Invalid block list entry"))?,
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid block list configuration"))?
    }
  }

  if !config["environmentVariables"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Environment variable configuration is not allowed in host configuration"
      ))?
    }
    if let Some(environment_variables_hash) = config["environmentVariables"].as_hash() {
      let environment_variables_hash_iter = environment_variables_hash.iter();
      for (var_name, var_value) in environment_variables_hash_iter {
        if var_name.as_str().is_none() || var_value.as_str().is_none() {
          Err(anyhow::anyhow!("Invalid environment variables"))?
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid environment variables"))?
    }
  }

  if !config["disableToHTTPSRedirect"].is_badvalue()
    && config["disableToHTTPSRedirect"].as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid HTTP to HTTPS redirect disabling option value"
    ))?
  }

  if !config["wwwredirect"].is_badvalue() && config["wwwredirect"].as_bool().is_none() {
    Err(anyhow::anyhow!(
      "Invalid to \"www.\" URL redirect disabling option value"
    ))?
  }

  if !config["customHeaders"].is_badvalue() {
    if let Some(custom_headers_hash) = config["customHeaders"].as_hash() {
      let custom_headers_hash_iter = custom_headers_hash.iter();
      for (header_name, header_value) in custom_headers_hash_iter {
        if let Some(header_name) = header_name.as_str() {
          if let Some(header_value) = header_value.as_str() {
            if HeaderValue::from_str(header_value).is_err()
              || HeaderName::from_str(header_name).is_err()
            {
              Err(anyhow::anyhow!("Invalid custom headers"))?
            }
          } else {
            Err(anyhow::anyhow!("Invalid custom headers"))?
          }
        } else {
          Err(anyhow::anyhow!("Invalid custom headers"))?
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid custom headers"))?
    }
  }

  if !config["rewriteMap"].is_badvalue() {
    if let Some(rewrite_map) = config["rewriteMap"].as_vec() {
      let rewrite_map_iter = rewrite_map.iter();
      for rewrite_map_entry_yaml in rewrite_map_iter {
        if !rewrite_map_entry_yaml.is_hash() {
          Err(anyhow::anyhow!("Invalid URL rewrite map"))?
        }
        if rewrite_map_entry_yaml["regex"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid URL rewrite map"))?
        }
        if rewrite_map_entry_yaml["replacement"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid URL rewrite map"))?
        }
        if !rewrite_map_entry_yaml["isNotFile"].is_badvalue()
          && rewrite_map_entry_yaml["isNotFile"].as_bool().is_none()
        {
          Err(anyhow::anyhow!("Invalid URL rewrite map"))?
        }
        if !rewrite_map_entry_yaml["isNotDirectory"].is_badvalue()
          && rewrite_map_entry_yaml["isNotDirectory"].as_bool().is_none()
        {
          Err(anyhow::anyhow!("Invalid URL rewrite map"))?
        }
        if !rewrite_map_entry_yaml["allowDoubleSlashes"].is_badvalue()
          && rewrite_map_entry_yaml["allowDoubleSlashes"]
            .as_bool()
            .is_none()
        {
          Err(anyhow::anyhow!("Invalid URL rewrite map"))?
        }
        if !rewrite_map_entry_yaml["last"].is_badvalue()
          && rewrite_map_entry_yaml["last"].as_bool().is_none()
        {
          Err(anyhow::anyhow!("Invalid URL rewrite map"))?
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid URL rewrite map"))?
    }
  }

  if !config["enableRewriteLogging"].is_badvalue()
    && config["enableRewriteLogging"].as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid URL rewrite logging enabling option value"
    ))?
  }

  if !config["disableTrailingSlashRedirects"].is_badvalue()
    && config["disableTrailingSlashRedirects"].as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid trailing slash redirect disabling option value"
    ))?
  }

  if !config["users"].is_badvalue() {
    if let Some(users) = config["users"].as_vec() {
      let users_iter = users.iter();
      for user_yaml in users_iter {
        if !user_yaml.is_hash() {
          Err(anyhow::anyhow!("Invalid user configuration"))?
        }
        if user_yaml["name"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid user configuration"))?
        }
        if user_yaml["pass"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid user configuration"))?
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid user configuration"))?
    }
  }

  if !config["nonStandardCodes"].is_badvalue() {
    if let Some(non_standard_codes) = config["nonStandardCodes"].as_vec() {
      let non_standard_codes_iter = non_standard_codes.iter();
      for non_standard_code_yaml in non_standard_codes_iter {
        if !non_standard_code_yaml.is_hash() {
          Err(anyhow::anyhow!(
            "Invalid non-standard status code configuration"
          ))?
        }
        if non_standard_code_yaml["scode"].as_i64().is_none() {
          Err(anyhow::anyhow!(
            "Invalid non-standard status code configuration"
          ))?
        }
        if !non_standard_code_yaml["regex"].is_badvalue()
          && non_standard_code_yaml["regex"].as_str().is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid non-standard status code configuration"
          ))?
        }
        if !non_standard_code_yaml["url"].is_badvalue()
          && non_standard_code_yaml["url"].as_str().is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid non-standard status code configuration"
          ))?
        }
        if non_standard_code_yaml["regex"].is_badvalue()
          && non_standard_code_yaml["url"].is_badvalue()
        {
          Err(anyhow::anyhow!(
            "Invalid non-standard status code configuration"
          ))?
        }
        if !non_standard_code_yaml["realm"].is_badvalue()
          && non_standard_code_yaml["realm"].as_str().is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid non-standard status code configuration"
          ))?
        }
        if !non_standard_code_yaml["disableBruteProtection"].is_badvalue()
          && non_standard_code_yaml["disableBruteProtection"]
            .as_bool()
            .is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid non-standard status code configuration"
          ))?
        }
        if !non_standard_code_yaml["userList"].is_badvalue() {
          if let Some(users) = non_standard_code_yaml["userList"].as_vec() {
            let users_iter = users.iter();
            for user_yaml in users_iter {
              if user_yaml.as_str().is_none() {
                Err(anyhow::anyhow!(
                  "Invalid non-standard status code configuration"
                ))?
              }
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid non-standard status code configuration"
            ))?
          }
        }
        if !non_standard_code_yaml["users"].is_badvalue() {
          if let Some(users) = non_standard_code_yaml["users"].as_vec() {
            let users_iter = users.iter();
            for user_yaml in users_iter {
              match user_yaml.as_str() {
                Some(user) => {
                  if !validate_ip(user) {
                    Err(anyhow::anyhow!(
                      "Invalid non-standard status code configuration"
                    ))?
                  }
                }
                None => Err(anyhow::anyhow!(
                  "Invalid non-standard status code configuration"
                ))?,
              }
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid non-standard status code configuration"
            ))?
          }
        }
      }
    } else {
      Err(anyhow::anyhow!(
        "Invalid non-standard status code configuration"
      ))?
    }
  }

  if !config["errorPages"].is_badvalue() {
    if let Some(error_pages) = config["errorPages"].as_vec() {
      let error_pages_iter = error_pages.iter();
      for error_page_yaml in error_pages_iter {
        if !error_page_yaml.is_hash() {
          Err(anyhow::anyhow!("Invalid custom error page configuration"))?
        }
        if error_page_yaml["scode"].as_i64().is_none() {
          Err(anyhow::anyhow!("Invalid custom error page configuration"))?
        }
        if error_page_yaml["path"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid custom error page configuration"))?
        }
      }
    } else {
      Err(anyhow::anyhow!("Invalid custom error page configuration"))?
    }
  }

  if !config["wwwroot"].is_badvalue() && config["wwwroot"].as_str().is_none() {
    Err(anyhow::anyhow!("Invalid webroot"))?
  }

  if !config["enableETag"].is_badvalue() && config["enableETag"].as_bool().is_none() {
    Err(anyhow::anyhow!("Invalid ETag enabling option"))?
  }

  if !config["enableCompression"].is_badvalue() && config["enableCompression"].as_bool().is_none() {
    Err(anyhow::anyhow!("Invalid HTTP compression enabling option"))?
  }

  if !config["enableDirectoryListing"].is_badvalue()
    && config["enableDirectoryListing"].as_bool().is_none()
  {
    Err(anyhow::anyhow!("Invalid directory listing enabling option"))?
  }

  if !config["enableAutomaticTLS"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Automatic TLS enabling configuration is not allowed in host configuration"
      ))?
    }
    if config["enableAutomaticTLS"].as_bool().is_none() {
      Err(anyhow::anyhow!(
        "Invalid automatic TLS enabling option value"
      ))?
    }
  }

  if !config["automaticTLSContactEmail"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Automatic TLS contact email address configuration is not allowed in host configuration"
      ))?
    }
    if config["automaticTLSContactEmail"].as_str().is_none() {
      Err(anyhow::anyhow!(
        "Invalid automatic TLS contact email address"
      ))?
    }
  }

  if !config["automaticTLSContactCacheDirectory"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Automatic TLS cache directory configuration is not allowed in host configuration"
      ))?
    }
    if config["automaticTLSContactCacheDirectory"]
      .as_str()
      .is_none()
    {
      Err(anyhow::anyhow!(
        "Invalid automatic TLS cache directory path"
      ))?
    }
  }

  if !config["automaticTLSLetsEncryptProduction"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Let's Encrypt production endpoint for automatic TLS enabling configuration is not allowed in host configuration"
      ))?
    }
    if config["automaticTLSLetsEncryptProduction"]
      .as_bool()
      .is_none()
    {
      Err(anyhow::anyhow!(
        "Invalid Let's Encrypt production endpoint for automatic TLS enabling option value"
      ))?
    }
  }

  if !config["timeout"].is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Server timeout configuration is not allowed in host configuration"
      ))?
    }
    if !config["timeout"].is_null() {
      if let Some(maximum_cache_response_size) = config["timeout"].as_i64() {
        if maximum_cache_response_size < 0 {
          Err(anyhow::anyhow!("Invalid server timeout"))?
        }
      } else {
        Err(anyhow::anyhow!("Invalid server timeout"))?
      }
    }
  }

  for module_optional_builtin in modules_optional_builtin.iter() {
    match module_optional_builtin as &str {
      #[cfg(feature = "rproxy")]
      "rproxy" => {
        if !config["proxyTo"].is_badvalue() {
          if let Some(proxy_urls) = config["proxyTo"].as_vec() {
            let proxy_urls_iter = proxy_urls.iter();
            for proxy_url_yaml in proxy_urls_iter {
              if proxy_url_yaml.as_str().is_none() {
                Err(anyhow::anyhow!("Invalid reverse proxy target URL value"))?
              }
            }
          } else if config["proxyTo"].as_str().is_none() {
            Err(anyhow::anyhow!("Invalid reverse proxy target URL value"))?
          }
        }

        if !config["secureProxyTo"].is_badvalue() {
          if let Some(proxy_urls) = config["secureProxyTo"].as_vec() {
            let proxy_urls_iter = proxy_urls.iter();
            for proxy_url_yaml in proxy_urls_iter {
              if proxy_url_yaml.as_str().is_none() {
                Err(anyhow::anyhow!(
                  "Invalid secure reverse proxy target URL value"
                ))?
              }
            }
          } else if config["secureProxyTo"].as_str().is_none() {
            Err(anyhow::anyhow!(
              "Invalid secure reverse proxy target URL value"
            ))?
          }
        }

        if !config["enableLoadBalancerHealthCheck"].is_badvalue()
          && config["enableLoadBalancerHealthCheck"].as_bool().is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid load balancer health check enabling option value"
          ))?
        }

        if !config["loadBalancerHealthCheckMaximumFails"].is_badvalue() {
          if let Some(window) = config["loadBalancerHealthCheckMaximumFails"].as_i64() {
            if window < 0 {
              Err(anyhow::anyhow!(
                "Invalid load balancer health check maximum fails value"
              ))?
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid load balancer health check maximum fails value"
            ))?
          }
        }

        if !config["loadBalancerHealthCheckWindow"].is_badvalue() {
          if !is_global {
            Err(anyhow::anyhow!(
              "Load balancer health check window configuration is not allowed in host configuration"
            ))?
          }
          if let Some(window) = config["loadBalancerHealthCheckWindow"].as_i64() {
            if window < 0 {
              Err(anyhow::anyhow!(
                "Invalid load balancer health check window value"
              ))?
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid load balancer health check window value"
            ))?
          }
        }

        if !config["disableProxyCertificateVerification"].is_badvalue()
          && config["disableProxyCertificateVerification"]
            .as_bool()
            .is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid proxy certificate verification disabling option value"
          ))?
        }
      }
      #[cfg(feature = "cache")]
      "cache" => {
        if !config["cacheVaryHeaders"].is_badvalue() {
          if let Some(modules) = config["cacheVaryHeaders"].as_vec() {
            let modules_iter = modules.iter();
            for module_name_yaml in modules_iter {
              if module_name_yaml.as_str().is_none() {
                Err(anyhow::anyhow!("Invalid varying cache header"))?
              }
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid varying cache headers configuration"
            ))?
          }
        }

        if !config["cacheIgnoreHeaders"].is_badvalue() {
          if let Some(modules) = config["cacheIgnoreHeaders"].as_vec() {
            let modules_iter = modules.iter();
            for module_name_yaml in modules_iter {
              if module_name_yaml.as_str().is_none() {
                Err(anyhow::anyhow!("Invalid ignored cache header"))?
              }
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid ignored cache headers configuration"
            ))?
          }
        }

        if !config["maximumCacheResponseSize"].is_badvalue()
          && !config["maximumCacheResponseSize"].is_null()
        {
          if let Some(maximum_cache_response_size) = config["maximumCacheResponseSize"].as_i64() {
            if maximum_cache_response_size < 0 {
              Err(anyhow::anyhow!("Invalid maximum cache response size"))?
            }
          } else {
            Err(anyhow::anyhow!("Invalid maximum cache response size"))?
          }
        }

        if !config["maximumCacheEntries"].is_badvalue() {
          if !is_global {
            Err(anyhow::anyhow!(
              "Maximum cache entries configuration is not allowed in host configuration"
            ))?
          }
          if !config["maximumCacheEntries"].is_null() {
            if let Some(maximum_cache_response_size) = config["maximumCacheEntries"].as_i64() {
              if maximum_cache_response_size < 0 {
                Err(anyhow::anyhow!("Invalid maximum cache entries"))?
              }
            } else {
              Err(anyhow::anyhow!("Invalid maximum cache entries"))?
            }
          }
        }
      }
      #[cfg(feature = "cgi")]
      "cgi" => {
        if !config["cgiScriptExtensions"].is_badvalue() {
          if let Some(cgi_script_extensions) = config["cgiScriptExtensions"].as_vec() {
            let cgi_script_extensions_iter = cgi_script_extensions.iter();
            for cgi_script_extension_yaml in cgi_script_extensions_iter {
              if cgi_script_extension_yaml.as_str().is_none() {
                Err(anyhow::anyhow!("Invalid CGI script extension"))?
              }
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid CGI script extension configuration"
            ))?
          }
        }

        if !config["cgiScriptInterpreters"].is_badvalue() {
          if let Some(cgi_script_interpreters) = config["cgiScriptInterpreters"].as_hash() {
            for (cgi_script_interpreter_extension_unknown, cgi_script_interpreter_params_unknown) in
              cgi_script_interpreters.iter()
            {
              if cgi_script_interpreter_extension_unknown.as_str().is_some() {
                if !cgi_script_interpreter_params_unknown.is_null() {
                  if let Some(cgi_script_interpreter_params) =
                    cgi_script_interpreter_params_unknown.as_vec()
                  {
                    let cgi_script_interpreter_params_iter = cgi_script_interpreter_params.iter();
                    for cgi_script_interpreter_param_yaml in cgi_script_interpreter_params_iter {
                      if cgi_script_interpreter_param_yaml.as_str().is_none() {
                        Err(anyhow::anyhow!("Invalid CGI script interpreter parameter"))?
                      }
                    }
                  } else {
                    Err(anyhow::anyhow!("Invalid CGI script interpreter parameters"))?
                  }
                }
              } else {
                Err(anyhow::anyhow!("Invalid CGI script interpreter extension"))?
              }
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid CGI script interpreter configuration"
            ))?
          }
        }
      }
      #[cfg(feature = "scgi")]
      "scgi" => {
        if !config["scgiTo"].is_badvalue() && config["scgiTo"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid SCGI target URL value"))?
        }

        if !config["scgiPath"].is_badvalue() && config["scgiPath"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid SCGI path"))?
        }
      }
      #[cfg(feature = "fcgi")]
      "fcgi" => {
        if !config["fcgiScriptExtensions"].is_badvalue() {
          if let Some(fastcgi_script_extensions) = config["fcgiScriptExtensions"].as_vec() {
            let fastcgi_script_extensions_iter = fastcgi_script_extensions.iter();
            for fastcgi_script_extension_yaml in fastcgi_script_extensions_iter {
              if fastcgi_script_extension_yaml.as_str().is_none() {
                Err(anyhow::anyhow!("Invalid CGI script extension"))?
              }
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid CGI script extension configuration"
            ))?
          }
        }

        if !config["fcgiTo"].is_badvalue() && config["fcgiTo"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid FastCGI target URL value"))?
        }

        if !config["fcgiPath"].is_badvalue() && config["fcgiPath"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid FastCGI path"))?
        }
      }
      #[cfg(feature = "fauth")]
      "fauth" => {
        if !config["authTo"].is_badvalue() && config["authTo"].as_str().is_none() {
          Err(anyhow::anyhow!(
            "Invalid forwarded authentication target URL value"
          ))?
        }

        if !config["forwardedAuthCopyHeaders"].is_badvalue() {
          if let Some(modules) = config["forwardedAuthCopyHeaders"].as_vec() {
            let modules_iter = modules.iter();
            for module_name_yaml in modules_iter {
              if module_name_yaml.as_str().is_none() {
                Err(anyhow::anyhow!(
                  "Invalid forwarded authentication response header to copy"
                ))?
              }
            }
          } else {
            Err(anyhow::anyhow!(
              "Invalid forwarded authentication response headers to copy configuration"
            ))?
          }
        }
      }
      _ => (),
    }
  }

  Ok(())
}

pub fn prepare_config_for_validation(
  config: &Yaml,
) -> Result<impl Iterator<Item = (Yaml, bool, bool)>, Box<dyn Error + Send + Sync>> {
  let mut vector = Vec::new();
  if let Some(global_config) = config["global"].as_hash() {
    let global_config_yaml = Yaml::Hash(global_config.clone());
    vector.push(global_config_yaml);
  }

  let mut vector2 = Vec::new();
  let mut vector3 = Vec::new();
  if !config["hosts"].is_badvalue() {
    if let Some(hosts) = config["hosts"].as_vec() {
      for host in hosts.iter() {
        if let Some(locations) = host["locations"].as_vec() {
          vector3.append(&mut locations.clone());
        }
      }
      vector2 = hosts.clone();
    } else {
      return Err(anyhow::anyhow!("Invalid virtual host configuration").into());
    }
  }

  let iter = vector
    .into_iter()
    .map(|item| (item, true, false))
    .chain(vector2.into_iter().map(|item| (item, false, false)))
    .chain(vector3.into_iter().map(|item| (item, false, true)));

  Ok(iter)
}
