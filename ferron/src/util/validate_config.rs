use crate::ferron_common::ServerConfig;
use hyper::header::{HeaderName, HeaderValue};
use std::collections::HashSet;
use std::error::Error;
use std::net::IpAddr;
use std::str::FromStr;
use yaml_rust2::{yaml, Yaml};

// Struct to store used configuration properties
struct UsedProperties<'a> {
  config: &'a ServerConfig,
  properties: HashSet<String>,
}

impl<'a> UsedProperties<'a> {
  fn new(config: &'a ServerConfig) -> Self {
    UsedProperties {
      config,
      properties: HashSet::new(),
    }
  }

  fn contains(&mut self, property: &str) -> bool {
    self.properties.insert(property.to_string());
    !self.config[property].is_badvalue()
  }

  fn unused(&self) -> Vec<String> {
    let empty_hashmap = yaml::Hash::new();
    let all_properties = self
      .config
      .as_hash()
      .unwrap_or(&empty_hashmap)
      .keys()
      .filter_map(|a| a.as_str().map(|a| a.to_string()));
    all_properties
      .filter(|item| !self.properties.contains(item))
      .collect()
  }
}

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
  is_error_config: bool,
  modules_optional_builtin: &[String],
) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
  let mut used_properties = UsedProperties::new(&config);

  let domain_badvalue = !used_properties.contains("domain");
  let ip_badvalue = !used_properties.contains("ip");

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

  if domain_badvalue && ip_badvalue && !is_global && !is_location && !is_error_config {
    Err(anyhow::anyhow!(
      "A host must either have IP address or domain name specified"
    ))?;
  }

  if used_properties.contains("scode") {
    if !is_error_config {
      Err(anyhow::anyhow!(
        "Status code configuration is only allowed in error configuration"
      ))?;
    }
    if config["scode"].as_i64().is_none() {
      Err(anyhow::anyhow!("Invalid status code"))?;
    }
  }

  if used_properties.contains("locations") {
    if is_location {
      Err(anyhow::anyhow!("Nested locations are not allowed"))?;
    } else if is_error_config {
      Err(anyhow::anyhow!(
        "The location configuration is not allowed in the error configuration"
      ))?;
    } else if is_global {
      Err(anyhow::anyhow!(
        "The location configuration is not allowed in the global configuration"
      ))?;
    }
  }

  if used_properties.contains("errorConfig") {
    if is_error_config {
      Err(anyhow::anyhow!("Nested error configuration is not allowed"))?;
    } else if is_location {
      Err(anyhow::anyhow!(
        "The error configuration is not allowed in the location configuration"
      ))?;
    } else if is_global {
      Err(anyhow::anyhow!(
        "The error configuration is not allowed in the global configuration"
      ))?;
    }
  }

  if used_properties.contains("loadModules") {
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

  if used_properties.contains("port") {
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

  if used_properties.contains("sport") {
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

  if used_properties.contains("secure") {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTPS enabling configuration is not allowed in host configuration"
      ))?
    }
    if config["secure"].as_bool().is_none() {
      Err(anyhow::anyhow!("Invalid HTTPS enabling option value"))?
    }
  }

  if used_properties.contains("enableHTTP2") {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTP/2 enabling configuration is not allowed in host configuration"
      ))?
    }
    if config["enableHTTP2"].as_bool().is_none() {
      Err(anyhow::anyhow!("Invalid HTTP/2 enabling option value"))?
    }
  }

  if used_properties.contains("enableHTTP3") {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTP/3 enabling configuration is not allowed in host configuration"
      ))?
    }
    if config["enableHTTP3"].as_bool().is_none() {
      Err(anyhow::anyhow!("Invalid HTTP/3 enabling option value"))?
    }
  }

  if used_properties.contains("logFilePath") {
    if !is_global {
      Err(anyhow::anyhow!(
        "Log file configuration is not allowed in host configuration"
      ))?
    }
    if config["logFilePath"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid log file path"))?
    }
  }

  if used_properties.contains("errorLogFilePath") {
    if !is_global {
      Err(anyhow::anyhow!(
        "Error log file configuration is not allowed in host configuration"
      ))?
    }
    if config["errorLogFilePath"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid error log file path"))?
    }
  }

  if used_properties.contains("cert") {
    if !is_global {
      Err(anyhow::anyhow!(
        "TLS certificate configuration is not allowed in host configuration"
      ))?
    }
    if config["cert"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid TLS certificate path"))?
    }
  }

  if used_properties.contains("key") {
    if !is_global {
      Err(anyhow::anyhow!(
        "Private key configuration is not allowed in host configuration"
      ))?
    }
    if config["key"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid private key path"))?
    }
  }

  if used_properties.contains("sni") {
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

  if used_properties.contains("http2Settings") {
    if !is_global {
      Err(anyhow::anyhow!(
        "HTTP/2 configuration is not allowed in host configuration"
      ))?
    }
    if config["http2Settings"].as_hash().is_some() {
      if let Some(initial_window_size) = config["http2Settings"]["initialWindowSize"].as_i64() {
        if !(0..=2_147_483_647).contains(&initial_window_size) {
          Err(anyhow::anyhow!("Invalid HTTP/2 initial window size"))?
        }
      }

      if let Some(max_frame_size) = config["http2Settings"]["maxFrameSize"].as_i64() {
        if !(16_384..=16_777_215).contains(&max_frame_size) {
          Err(anyhow::anyhow!("Invalid HTTP/2 max frame size"))?
        }
      }

      if let Some(max_concurrent_streams) = config["http2Settings"]["maxConcurrentStreams"].as_i64()
      {
        if max_concurrent_streams < 0 {
          Err(anyhow::anyhow!("Invalid HTTP/2 max concurrent streams"))?
        }
      }

      if let Some(max_header_list_size) = config["http2Settings"]["maxHeaderListSize"].as_i64() {
        if max_header_list_size < 0 {
          Err(anyhow::anyhow!("Invalid HTTP/2 max header list size"))?
        }
      }

      if !config["http2Settings"]["enableConnectProtocol"].is_badvalue()
        && config["http2Settings"]["enableConnectProtocol"]
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

  if used_properties.contains("useClientCertificate") {
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

  if used_properties.contains("cipherSuite") {
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

  if used_properties.contains("ecdhCurve") {
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

  if used_properties.contains("tlsMinVersion") {
    if !is_global {
      Err(anyhow::anyhow!(
        "Minimum TLS version is not allowed in host configuration"
      ))?
    }
    if config["tlsMinVersion"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid minimum TLS version"))?
    }
  }

  if used_properties.contains("tlsMaxVersion") {
    if !is_global {
      Err(anyhow::anyhow!(
        "Maximum TLS version is not allowed in host configuration"
      ))?
    }
    if config["tlsMaxVersion"].as_str().is_none() {
      Err(anyhow::anyhow!("Invalid maximum TLS version"))?
    }
  }

  if used_properties.contains("enableOCSPStapling") {
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

  if used_properties.contains("serverAdministratorEmail")
    && config["serverAdministratorEmail"].as_str().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid server administrator email address"
    ))?
  }

  if used_properties.contains("enableIPSpoofing") && config["enableIPSpoofing"].as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid X-Forwarded-For enabling option value"
    ))?
  }

  if used_properties.contains("disableNonEncryptedServer") {
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

  if used_properties.contains("blocklist") {
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

  if used_properties.contains("environmentVariables") {
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

  if used_properties.contains("disableToHTTPSRedirect")
    && config["disableToHTTPSRedirect"].as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid HTTP to HTTPS redirect disabling option value"
    ))?
  }

  if used_properties.contains("wwwredirect") && config["wwwredirect"].as_bool().is_none() {
    Err(anyhow::anyhow!(
      "Invalid to \"www.\" URL redirect disabling option value"
    ))?
  }

  if used_properties.contains("customHeaders") {
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

  if used_properties.contains("rewriteMap") {
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

  if used_properties.contains("enableRewriteLogging")
    && config["enableRewriteLogging"].as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid URL rewrite logging enabling option value"
    ))?
  }

  if used_properties.contains("disableTrailingSlashRedirects")
    && config["disableTrailingSlashRedirects"].as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid trailing slash redirect disabling option value"
    ))?
  }

  if used_properties.contains("users") {
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

  if used_properties.contains("nonStandardCodes") {
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

  if used_properties.contains("errorPages") {
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

  if used_properties.contains("wwwroot") && config["wwwroot"].as_str().is_none() {
    Err(anyhow::anyhow!("Invalid webroot"))?
  }

  if used_properties.contains("enableETag") && config["enableETag"].as_bool().is_none() {
    Err(anyhow::anyhow!("Invalid ETag enabling option"))?
  }

  if used_properties.contains("enableCompression")
    && config["enableCompression"].as_bool().is_none()
  {
    Err(anyhow::anyhow!("Invalid HTTP compression enabling option"))?
  }

  if used_properties.contains("enableDirectoryListing")
    && config["enableDirectoryListing"].as_bool().is_none()
  {
    Err(anyhow::anyhow!("Invalid directory listing enabling option"))?
  }

  if used_properties.contains("enableAutomaticTLS")
    && config["enableAutomaticTLS"].as_bool().is_none()
  {
    Err(anyhow::anyhow!(
      "Invalid automatic TLS enabling option value"
    ))?
  }

  if used_properties.contains("useAutomaticTLSHTTPChallenge") {
    if !is_global {
      Err(anyhow::anyhow!(
        "Automatic TLS HTTP challenge enabling configuration is not allowed in host configuration"
      ))?
    }
    if config["useAutomaticTLSHTTPChallenge"].as_bool().is_none() {
      Err(anyhow::anyhow!(
        "Invalid automatic TLS HTTP challenge enabling option value"
      ))?
    }
  }

  if used_properties.contains("automaticTLSContactEmail") {
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

  if used_properties.contains("automaticTLSContactCacheDirectory") {
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

  if used_properties.contains("automaticTLSLetsEncryptProduction") {
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

  if used_properties.contains("timeout") {
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
        if used_properties.contains("proxyTo") {
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

        if used_properties.contains("secureProxyTo") {
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

        if used_properties.contains("enableLoadBalancerHealthCheck")
          && config["enableLoadBalancerHealthCheck"].as_bool().is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid load balancer health check enabling option value"
          ))?
        }

        if used_properties.contains("loadBalancerHealthCheckMaximumFails") {
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

        if used_properties.contains("loadBalancerHealthCheckWindow") {
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

        if used_properties.contains("disableProxyCertificateVerification")
          && config["disableProxyCertificateVerification"]
            .as_bool()
            .is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid proxy certificate verification disabling option value"
          ))?
        }

        if used_properties.contains("proxyInterceptErrors")
          && config["proxyInterceptErrors"].as_bool().is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid reverse proxy error interception option value"
          ))?
        }
      }
      #[cfg(feature = "cache")]
      "cache" => {
        if used_properties.contains("cacheVaryHeaders") {
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

        if used_properties.contains("cacheIgnoreHeaders") {
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

        if used_properties.contains("maximumCacheResponseSize")
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

        if used_properties.contains("maximumCacheEntries") {
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
        if used_properties.contains("cgiScriptExtensions") {
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

        if used_properties.contains("cgiScriptInterpreters") {
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
        if used_properties.contains("scgiTo") && config["scgiTo"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid SCGI target URL value"))?
        }

        if used_properties.contains("scgiPath") && config["scgiPath"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid SCGI path"))?
        }
      }
      #[cfg(feature = "fcgi")]
      "fcgi" => {
        if used_properties.contains("fcgiScriptExtensions") {
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

        if used_properties.contains("fcgiTo") && config["fcgiTo"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid FastCGI target URL value"))?
        }

        if used_properties.contains("fcgiPath") && config["fcgiPath"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid FastCGI path"))?
        }
      }
      #[cfg(feature = "fauth")]
      "fauth" => {
        if used_properties.contains("authTo") && config["authTo"].as_str().is_none() {
          Err(anyhow::anyhow!(
            "Invalid forwarded authentication target URL value"
          ))?
        }

        if used_properties.contains("forwardedAuthCopyHeaders") {
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
      #[cfg(feature = "wsgi")]
      "wsgi" => {
        if used_properties.contains("wsgiApplicationPath")
          && config["wsgiApplicationPath"].as_str().is_none()
        {
          Err(anyhow::anyhow!("Invalid path to the WSGI application"))?
        }

        if used_properties.contains("wsgiPath") && config["wsgiPath"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid WSGI request base path"))?
        }

        if used_properties.contains("wsgiClearModuleImportPath") {
          if !is_global {
            Err(anyhow::anyhow!(
              "WSGI Python module import path clearing option is not allowed in host configuration"
            ))?
          }
          if config["wsgiClearModuleImportPath"].as_bool().is_none() {
            Err(anyhow::anyhow!(
              "Invalid WSGI Python module import path clearing option value"
            ))?
          }
        }
      }
      #[cfg(feature = "wsgid")]
      "wsgid" => {
        if used_properties.contains("wsgidApplicationPath")
          && config["wsgidApplicationPath"].as_str().is_none()
        {
          Err(anyhow::anyhow!(
            "Invalid path to the WSGI (with pre-forked process pool) application"
          ))?
        }

        if used_properties.contains("wsgidPath") && config["wsgidPath"].as_str().is_none() {
          Err(anyhow::anyhow!(
            "Invalid WSGI (with pre-forked process pool) request base path"
          ))?
        }
      }
      #[cfg(feature = "asgi")]
      "asgi" => {
        if used_properties.contains("asgiApplicationPath")
          && config["asgiApplicationPath"].as_str().is_none()
        {
          Err(anyhow::anyhow!("Invalid path to the ASGI application"))?
        }

        if used_properties.contains("asgiPath") && config["asgiPath"].as_str().is_none() {
          Err(anyhow::anyhow!("Invalid ASGI request base path"))?
        }

        if used_properties.contains("asgiClearModuleImportPath") {
          if !is_global {
            Err(anyhow::anyhow!(
              "ASGI Python module import path clearing option is not allowed in host configuration"
            ))?
          }
          if config["asgiClearModuleImportPath"].as_bool().is_none() {
            Err(anyhow::anyhow!(
              "Invalid ASGI Python module import path clearing option value"
            ))?
          }
        }
      }
      _ => (),
    }
  }

  Ok(used_properties.unused())
}

pub fn prepare_config_for_validation(
  config: &Yaml,
) -> Result<impl Iterator<Item = (Yaml, bool, bool, bool)>, Box<dyn Error + Send + Sync>> {
  let mut vector = Vec::new();
  if let Some(global_config) = config["global"].as_hash() {
    let global_config_yaml = Yaml::Hash(global_config.clone());
    vector.push(global_config_yaml);
  }

  let mut vector2 = Vec::new();
  let mut vector3 = Vec::new();
  let mut vector4 = Vec::new();
  if !config["hosts"].is_badvalue() {
    if let Some(hosts) = config["hosts"].as_vec() {
      for host in hosts.iter() {
        if !host["errorConfig"].is_badvalue() {
          if let Some(error_configs) = host["errorConfig"].as_vec() {
            vector3.append(&mut error_configs.clone());
          } else {
            return Err(anyhow::anyhow!("Invalid location configuration").into());
          }
        }
        if !host["locations"].is_badvalue() {
          if let Some(locations) = host["locations"].as_vec() {
            vector4.append(&mut locations.clone());
          } else {
            return Err(anyhow::anyhow!("Invalid location configuration").into());
          }
        }
      }
      vector2 = hosts.clone();
    } else {
      return Err(anyhow::anyhow!("Invalid virtual host configuration").into());
    }
  }

  let iter = vector
    .into_iter()
    .map(|item| (item, true, false, false))
    .chain(vector2.into_iter().map(|item| (item, false, false, false)))
    .chain(vector3.into_iter().map(|item| (item, false, true, false)))
    .chain(vector4.into_iter().map(|item| (item, false, false, true)));

  Ok(iter)
}
