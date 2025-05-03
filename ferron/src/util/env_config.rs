use std::env;
use yaml_rust2::Yaml;

/// Apply environment variable overrides with prefix FERRON_ to the provided YAML configuration.
/// Maps directly to the fields in the global section of ferron.yaml.
pub fn apply_env_vars_to_config(yaml_config: &mut Yaml) {
  let global_hash = match yaml_config["global"].as_mut_hash() {
    Some(h) => h,
    None => return,
  };

  // Port settings
  if let Ok(port_val) = env::var("FERRON_PORT") {
    if let Ok(port) = port_val.parse::<i64>() {
      global_hash.insert(Yaml::String("port".into()), Yaml::Integer(port));
    } else {
      // Handle port as string for address:port format
      global_hash.insert(Yaml::String("port".into()), Yaml::String(port_val));
    }
  }

  if let Ok(sport_val) = env::var("FERRON_SPORT") {
    if let Ok(sport) = sport_val.parse::<i64>() {
      global_hash.insert(Yaml::String("sport".into()), Yaml::Integer(sport));
    } else {
      // Handle sport as string for address:port format
      global_hash.insert(Yaml::String("sport".into()), Yaml::String(sport_val));
    }
  }

  // HTTP/2 settings - only add if at least one HTTP/2 variable is set
  let http2_initial_window = env::var("FERRON_HTTP2_INITIAL_WINDOW_SIZE")
    .ok()
    .and_then(|val| val.parse::<i64>().ok());
  let http2_max_frame = env::var("FERRON_HTTP2_MAX_FRAME_SIZE")
    .ok()
    .and_then(|val| val.parse::<i64>().ok());
  let http2_max_streams = env::var("FERRON_HTTP2_MAX_CONCURRENT_STREAMS")
    .ok()
    .and_then(|val| val.parse::<i64>().ok());
  let http2_max_header = env::var("FERRON_HTTP2_MAX_HEADER_LIST_SIZE")
    .ok()
    .and_then(|val| val.parse::<i64>().ok());
  let http2_enable_connect = env::var("FERRON_HTTP2_ENABLE_CONNECT_PROTOCOL")
    .ok()
    .map(|val| matches!(val.to_ascii_lowercase().as_str(), "1" | "true" | "yes"));

  // Only create the http2Settings hash if at least one setting is present
  if http2_initial_window.is_some()
    || http2_max_frame.is_some()
    || http2_max_streams.is_some()
    || http2_max_header.is_some()
    || http2_enable_connect.is_some()
  {
    let mut http2_hash = yaml_rust2::yaml::Hash::new();

    // Add settings if they exist
    if let Some(size) = http2_initial_window {
      http2_hash.insert(
        Yaml::String("initialWindowSize".into()),
        Yaml::Integer(size),
      );
    }

    if let Some(size) = http2_max_frame {
      http2_hash.insert(Yaml::String("maxFrameSize".into()), Yaml::Integer(size));
    }

    if let Some(streams) = http2_max_streams {
      http2_hash.insert(
        Yaml::String("maxConcurrentStreams".into()),
        Yaml::Integer(streams),
      );
    }

    if let Some(size) = http2_max_header {
      http2_hash.insert(
        Yaml::String("maxHeaderListSize".into()),
        Yaml::Integer(size),
      );
    }

    if let Some(enable) = http2_enable_connect {
      http2_hash.insert(
        Yaml::String("enableConnectProtocol".into()),
        Yaml::Boolean(enable),
      );
    }

    // Only add the http2Settings to global if we have settings
    if !http2_hash.is_empty() {
      global_hash.insert(Yaml::String("http2Settings".into()), Yaml::Hash(http2_hash));
    }
  }

  // Log paths
  if let Ok(path) = env::var("FERRON_LOG_FILE_PATH") {
    global_hash.insert(Yaml::String("logFilePath".into()), Yaml::String(path));
  }

  if let Ok(path) = env::var("FERRON_ERROR_LOG_FILE_PATH") {
    global_hash.insert(Yaml::String("errorLogFilePath".into()), Yaml::String(path));
  }

  // TLS/HTTPS settings
  if let Ok(cert) = env::var("FERRON_CERT") {
    global_hash.insert(Yaml::String("cert".into()), Yaml::String(cert));
  }

  if let Ok(key) = env::var("FERRON_KEY") {
    global_hash.insert(Yaml::String("key".into()), Yaml::String(key));
  }

  if let Ok(min_ver) = env::var("FERRON_TLS_MIN_VERSION") {
    global_hash.insert(Yaml::String("tlsMinVersion".into()), Yaml::String(min_ver));
  }

  if let Ok(max_ver) = env::var("FERRON_TLS_MAX_VERSION") {
    global_hash.insert(Yaml::String("tlsMaxVersion".into()), Yaml::String(max_ver));
  }

  // Boolean settings
  if let Ok(v) = env::var("FERRON_SECURE") {
    let enable = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
    global_hash.insert(Yaml::String("secure".into()), Yaml::Boolean(enable));
  }

  if let Ok(v) = env::var("FERRON_ENABLE_HTTP2") {
    let enable = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
    global_hash.insert(Yaml::String("enableHTTP2".into()), Yaml::Boolean(enable));
  }

  if let Ok(v) = env::var("FERRON_ENABLE_HTTP3") {
    let enable = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
    global_hash.insert(Yaml::String("enableHTTP3".into()), Yaml::Boolean(enable));
  }

  if let Ok(v) = env::var("FERRON_DISABLE_NON_ENCRYPTED_SERVER") {
    let enable = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
    global_hash.insert(
      Yaml::String("disableNonEncryptedServer".into()),
      Yaml::Boolean(enable),
    );
  }

  if let Ok(v) = env::var("FERRON_ENABLE_OCSP_STAPLING") {
    let enable = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
    global_hash.insert(
      Yaml::String("enableOCSPStapling".into()),
      Yaml::Boolean(enable),
    );
  }

  if let Ok(v) = env::var("FERRON_ENABLE_DIRECTORY_LISTING") {
    let enable = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
    global_hash.insert(
      Yaml::String("enableDirectoryListing".into()),
      Yaml::Boolean(enable),
    );
  }

  if let Ok(v) = env::var("FERRON_ENABLE_COMPRESSION") {
    let enable = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
    global_hash.insert(
      Yaml::String("enableCompression".into()),
      Yaml::Boolean(enable),
    );
  }

  // Module loading
  if let Ok(list) = env::var("FERRON_LOAD_MODULES") {
    let arr: Vec<Yaml> = list
      .split(',')
      .filter_map(|s| {
        let t = s.trim();
        if t.is_empty() {
          None
        } else {
          Some(Yaml::String(t.to_string()))
        }
      })
      .collect();
    if !arr.is_empty() {
      global_hash.insert(Yaml::String("loadModules".into()), Yaml::Array(arr));
    }
  }

  // IP blocklist
  if let Ok(list) = env::var("FERRON_BLOCKLIST") {
    let arr: Vec<Yaml> = list
      .split(',')
      .filter_map(|s| {
        let t = s.trim();
        if t.is_empty() {
          None
        } else {
          Some(Yaml::String(t.to_string()))
        }
      })
      .collect();
    if !arr.is_empty() {
      global_hash.insert(Yaml::String("blocklist".into()), Yaml::Array(arr));
    }
  }

  // SNI configuration
  if let Ok(sni_hosts) = env::var("FERRON_SNI_HOSTS") {
    let hosts: Vec<&str> = sni_hosts
      .split(',')
      .map(|s| s.trim())
      .filter(|s| !s.is_empty())
      .collect();

    if !hosts.is_empty() {
      let mut sni_hash = yaml_rust2::yaml::Hash::new();

      for host in hosts {
        let cert_env_var = format!(
          "FERRON_SNI_{}_CERT",
          host
            .replace('.', "_")
            .replace('*', "WILDCARD")
            .to_uppercase()
        );
        let key_env_var = format!(
          "FERRON_SNI_{}_KEY",
          host
            .replace('.', "_")
            .replace('*', "WILDCARD")
            .to_uppercase()
        );

        if let (Ok(cert), Ok(key)) = (env::var(&cert_env_var), env::var(&key_env_var)) {
          let mut host_hash = yaml_rust2::yaml::Hash::new();
          host_hash.insert(Yaml::String("cert".into()), Yaml::String(cert));
          host_hash.insert(Yaml::String("key".into()), Yaml::String(key));
          sni_hash.insert(Yaml::String(host.to_string()), Yaml::Hash(host_hash));
        }
      }

      if !sni_hash.is_empty() {
        global_hash.insert(Yaml::String("sni".into()), Yaml::Hash(sni_hash));
      }
    }
  }

  // Environment variables for processes
  if let Ok(env_list) = env::var("FERRON_ENV_VARS") {
    let vars: Vec<&str> = env_list
      .split(',')
      .map(|s| s.trim())
      .filter(|s| !s.is_empty())
      .collect();

    if !vars.is_empty() {
      let mut env_hash = yaml_rust2::yaml::Hash::new();

      for var_name in vars {
        let env_var = format!("FERRON_ENV_{}", var_name.to_uppercase());

        if let Ok(value) = env::var(&env_var) {
          env_hash.insert(Yaml::String(var_name.to_string()), Yaml::String(value));
        }
      }

      if !env_hash.is_empty() {
        global_hash.insert(
          Yaml::String("environmentVariables".into()),
          Yaml::Hash(env_hash),
        );
      }
    }
  }
}

/// Return messages describing which env vars starting with FERRON_ are set (for logging).
pub fn log_env_var_overrides() -> Vec<String> {
  env::vars()
    .filter(|(k, _)| k.starts_with("FERRON_"))
    .map(|(k, v)| format!("Environment override: {}={}", k, v))
    .collect()
}
