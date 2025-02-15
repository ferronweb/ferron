use hyper::header::{HeaderName, HeaderValue};
use project_karpacz_common::ServerConfigRoot;
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
  config: &ServerConfigRoot,
  is_global: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  let domain_badvalue = config.get("domain").is_badvalue();
  let ip_badvalue = config.get("ip").is_badvalue();

  if !domain_badvalue && config.get("domain").as_str().is_none() {
    Err(anyhow::anyhow!("Invalid domain name"))?
  }

  if !ip_badvalue {
    match config.get("ip").as_str() {
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

  if domain_badvalue && ip_badvalue && !is_global {
    Err(anyhow::anyhow!(
      "A host must either have IP address or domain name specified"
    ))?;
  }

  if !config.get("loadModules").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Module configuration is not allowed in host configuration"
      ))?
    }
    if let Some(modules) = config.get("loadModules").as_vec() {
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

  if !config.get("port").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTP port configuration is not allowed in host configuration"
      ))?
    }
    if let Some(port) = config.get("port").as_i64() {
      if !(0..=65535).contains(&port) {
        Err(anyhow::anyhow!("Invalid HTTP port"))?
      }
    } else if config.get("port").as_str().is_none() {
      Err(anyhow::anyhow!("Invalid HTTP port"))?
    }
  }

  if !config.get("sport").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTPS port configuration is not allowed in host configuration"
      ))?
    }
    if let Some(port) = config.get("sport").as_i64() {
      if !(0..=65535).contains(&port) {
        Err(anyhow::anyhow!("Invalid HTTPS port"))?
      }
    } else if config.get("sport").as_str().is_none() {
      Err(anyhow::anyhow!("Invalid HTTPS port"))?
    }
  }

  if !config.get("secure").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTPS enabling configuration is not allowed in host configuration"
      ))?
    }
    if config.get("secure").as_bool().is_none() {
      Err(anyhow::anyhow!("Invalid HTTPS enabling option value"))?
    }
  }

  if !config.get("enableHTTP2").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTP/2 enabling configuration is not allowed in host configuration"
      ))?
    }
    if config.get("enableHTTP2").as_bool().is_none() {
      Err(anyhow::anyhow!("Invalid HTTP/2 enabling option value"))?
    }
  }

  if !config.get("logFilePath").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Log file configuration is not allowed in host configuration"
      ))?
    }
    if config.get("logFilePath").as_str().is_none() {
      Err(anyhow::anyhow!("Invalid log file path"))?
    }
  }

  if !config.get("errorLogFilePath").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Error log file configuration is not allowed in host configuration"
      ))?
    }
    if config.get("errorLogFilePath").as_str().is_none() {
      Err(anyhow::anyhow!("Invalid error log file path"))?
    }
  }

  if !config.get("cert").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "TLS certificate configuration is not allowed in host configuration"
      ))?
    }
    if config.get("cert").as_str().is_none() {
      Err(anyhow::anyhow!("Invalid TLS certificate path"))?
    }
  }

  if !config.get("key").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Private key configuration is not allowed in host configuration"
      ))?
    }
    if config.get("key").as_str().is_none() {
      Err(anyhow::anyhow!("Invalid private key path"))?
    }
  }

  if !config.get("sni").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "SNI configuration is not allowed in host configuration"
      ))?
    }
    if let Some(sni) = config.get("sni").as_hash() {
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

  if !config.get("http2Options").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTP/2 configuration is not allowed in host configuration"
      ))?
    }
    if config.get("http2Options").as_hash().is_some() {
      if let Some(initial_window_size) = config.get("http2Options")["initialWindowSize"].as_i64() {
        if !(0..=2_147_483_647).contains(&initial_window_size) {
          Err(anyhow::anyhow!("Invalid HTTP/2 initial window size"))?
        }
      }

      if let Some(max_frame_size) = config.get("http2Options")["maxFrameSize"].as_i64() {
        if !(16_384..=16_777_215).contains(&max_frame_size) {
          Err(anyhow::anyhow!("Invalid HTTP/2 max frame size"))?
        }
      }

      if let Some(max_concurrent_streams) =
        config.get("http2Options")["maxConcurrentStreams"].as_i64()
      {
        if max_concurrent_streams < 0 {
          Err(anyhow::anyhow!("Invalid HTTP/2 max concurrent streams"))?
        }
      }

      if let Some(max_header_list_size) = config.get("http2Options")["maxHeaderListSize"].as_i64() {
        if max_header_list_size < 0 {
          Err(anyhow::anyhow!("Invalid HTTP/2 max header list size"))?
        }
      }

      if !config.get("http2Options")["enableConnectProtocol"].is_badvalue()
        && config.get("http2Options")["enableConnectProtocol"]
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

  if !config.get("useClientCertificate").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Client certificate verfication enabling option is not allowed in host configuration"
      ))?
    }
    if config.get("useClientCertificate").as_bool().is_none() {
      Err(anyhow::anyhow!(
        "Invalid client certificate verification enabling option value"
      ))?
    }
  }

  if !config.get("cipherSuite").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Cipher suite configuration is not allowed in host configuration"
      ))?
    }
    if let Some(cipher_suites) = config.get("cipherSuite").as_vec() {
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

  if !config.get("ecdhCurve").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "ECDH curve configuration is not allowed in host configuration"
      ))?
    }
    if let Some(ecdh_curves) = config.get("ecdhCurve").as_vec() {
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

  if !config.get("tlsMinVersion").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Minimum TLS version is not allowed in host configuration"
      ))?
    }
    if config.get("tlsMinVersion").as_str().is_none() {
      Err(anyhow::anyhow!("Invalid minimum TLS version"))?
    }
  }

  if !config.get("tlsMaxVersion").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Maximum TLS version is not allowed in host configuration"
      ))?
    }
    if config.get("tlsMaxVersion").as_str().is_none() {
      Err(anyhow::anyhow!("Invalid maximum TLS version"))?
    }
  }

  if !config.get("enableOCSPStapling").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "OCSP stapling enabling option is not allowed in host configuration"
      ))?
    }
    if config.get("enableOCSPStapling").as_bool().is_none() {
      Err(anyhow::anyhow!(
        "Invalid OCSP stapling enabling option value"
      ))?
    }
  }

  if !config.get("serverAdministratorEmail").is_badvalue()
    && config.get("serverAdministratorEmail").as_str().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid server administrator email address"
    ))?
  }

  if !config.get("enableIPSpoofing").is_badvalue()
    && config.get("enableIPSpoofing").as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid X-Forwarded-For enabling option value"
    ))?
  }

  if !config.get("disableNonEncryptedServer").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Non-encrypted server disabling option is not allowed in host configuration"
      ))?
    }
    if config.get("disableNonEncryptedServer").as_bool().is_none() {
      Err(anyhow::anyhow!(
        "Invalid non-encrypted server disabling option value"
      ))?
    }
  }

  if !config.get("blocklist").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Block list configuration is not allowed in host configuration"
      ))?
    }
    if let Some(blocklist) = config.get("blocklist").as_vec() {
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

  if !config.get("environmentVariables").is_badvalue() {
    if !is_global {
      Err(anyhow::anyhow!(
        "Environment variable configuration is not allowed in host configuration"
      ))?
    }
    if let Some(environment_variables_hash) = config.get("environmentVariables").as_hash() {
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

  if !config.get("disableToHTTPSRedirect").is_badvalue()
    && config.get("disableToHTTPSRedirect").as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid HTTP to HTTPS redirect disabling option value"
    ))?
  }

  if !config.get("wwwredirect").is_badvalue() && config.get("wwwredirect").as_bool().is_none() {
    Err(anyhow::anyhow!(
      "Invalid to \"www.\" URL redirect disabling option value"
    ))?
  }

  if !config.get("customHeaders").is_badvalue() {
    if let Some(custom_headers_hash) = config.get("customHeaders").as_hash() {
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

  if !config.get("rewriteMap").is_badvalue() {
    if let Some(rewrite_map) = config.get("rewriteMap").as_vec() {
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

  if !config.get("enableRewriteLogging").is_badvalue()
    && config.get("enableRewriteLogging").as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid URL rewrite logging enabling option value"
    ))?
  }

  if !config.get("disableTrailingSlashRedirects").is_badvalue()
    && config
      .get("disableTrailingSlashRedirects")
      .as_bool()
      .is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid trailing slash redirect disabling option value"
    ))?
  }

  if !config.get("users").is_badvalue() {
    if let Some(users) = config.get("users").as_vec() {
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

  if !config.get("nonStandardCodes").is_badvalue() {
    if let Some(non_standard_codes) = config.get("nonStandardCodes").as_vec() {
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
        if !non_standard_code_yaml["non_standard_codeList"].is_badvalue() {
          if let Some(users) = non_standard_code_yaml["non_standard_codeList"].as_vec() {
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
        if !non_standard_code_yaml["non_standard_codes"].is_badvalue() {
          if let Some(users) = non_standard_code_yaml["non_standard_codes"].as_vec() {
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

  if !config.get("errorPages").is_badvalue() {
    if let Some(error_pages) = config.get("errorPages").as_vec() {
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

  if !config.get("wwwroot").is_badvalue() && config.get("wwwroot").as_str().is_none() {
    Err(anyhow::anyhow!("Invalid webroot"))?
  }

  if !config.get("enableETag").is_badvalue() && config.get("enableETag").as_bool().is_none() {
    Err(anyhow::anyhow!("Invalid ETag enabling option"))?
  }

  if !config.get("enableCompression").is_badvalue()
    && config.get("enableCompression").as_bool().is_none()
  {
    Err(anyhow::anyhow!("Invalid HTTP compression enabling option"))?
  }

  if !config.get("enableDirectoryListing").is_badvalue()
    && config.get("enableDirectoryListing").as_bool().is_none()
  {
    Err(anyhow::anyhow!("Invalid directory listing enabling option"))?
  }

  Ok(())
}

pub fn prepare_config_for_validation(
  config: &Yaml,
) -> Result<impl Iterator<Item = (Yaml, bool)>, Box<dyn Error + Send + Sync>> {
  let mut vector = Vec::new();
  if let Some(global_config) = config["global"].as_hash() {
    let global_config_yaml = Yaml::Hash(global_config.clone());
    vector.push(global_config_yaml);
  }

  let mut vector2 = Vec::new();
  if !config["hosts"].is_badvalue() {
    if let Some(hosts) = config["hosts"].as_vec() {
      vector2 = hosts.clone();
    } else {
      return Err(anyhow::anyhow!("Invalid virtual host configuration").into());
    }
  }

  let iter = vector
    .into_iter()
    .map(|item| (item, true))
    .chain(vector2.into_iter().map(|item| (item, false)));

  Ok(iter)
}
