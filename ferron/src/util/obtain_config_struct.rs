use std::error::Error;
use std::net::IpAddr;
use std::sync::Arc;

use crate::ferron_common::ServerConfig;
use crate::ferron_util::ip_match::ip_match;
use crate::ferron_util::match_hostname::match_hostname;
use crate::ferron_util::match_location::match_location;

pub struct ObtainConfigStruct<T> {
  global: Option<T>,
  host: Arc<Vec<ObtainConfigStructHost<T>>>,
}

struct ObtainConfigStructHost<T> {
  domain: Option<String>,
  ip: Option<String>,
  data: Option<T>,
  locations: Vec<ObtainConfigStructLocation<T>>,
  error_configs: Vec<ObtainConfigStructErrorConfig<T>>,
}

struct ObtainConfigStructLocation<T> {
  path: String,
  data: Option<T>,
}

struct ObtainConfigStructErrorConfig<T> {
  scode: Option<u16>,
  data: Option<T>,
}

impl<T> ObtainConfigStruct<T> {
  pub fn new(
    config: &ServerConfig,
    mut execute_fn: impl FnMut(&ServerConfig) -> Result<Option<T>, Box<dyn Error + Send + Sync>>,
  ) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let global_struct = execute_fn(&config["global"])?;
    let mut host_structs = Vec::new();

    if let Some(hosts) = config["hosts"].as_vec() {
      for host_yaml in hosts.iter() {
        let domain = host_yaml["domain"].as_str().map(String::from);
        let ip = host_yaml["ip"].as_str().map(String::from);
        let mut error_configs = Vec::new();
        let mut locations = Vec::new();
        if let Some(error_configs_yaml) = host_yaml["errorConfig"].as_vec() {
          for error_config_yaml in error_configs_yaml.iter() {
            let scode = error_config_yaml["scode"].as_i64().map(|s| s as u16);
            error_configs.push(ObtainConfigStructErrorConfig {
              scode,
              data: execute_fn(error_config_yaml)?,
            });
          }
        }
        if let Some(locations_yaml) = host_yaml["locations"].as_vec() {
          for location_yaml in locations_yaml.iter() {
            if let Some(path_str) = location_yaml["path"].as_str() {
              let path = String::from(path_str);
              locations.push(ObtainConfigStructLocation {
                path,
                data: execute_fn(location_yaml)?,
              });
            }
          }
        }
        host_structs.push(ObtainConfigStructHost {
          domain,
          ip,
          data: execute_fn(host_yaml)?,
          locations,
          error_configs,
        });
      }
    }

    Ok(ObtainConfigStruct {
      global: global_struct,
      host: Arc::new(host_structs),
    })
  }
}

impl<T> ObtainConfigStruct<T>
where
  T: Clone,
{
  pub fn obtain(
    &mut self,
    hostname: Option<&str>,
    ip: IpAddr,
    request_url: &str,
    status_code: Option<u16>,
  ) -> Option<T> {
    // Use .take() instead of .clone(), since the values in Options will only be used once.
    let mut data = self.global.take();

    // Should have used a HashMap instead of iterating over an array for better performance...
    for host in self.host.iter() {
      if match_hostname(
        match &host.domain {
          Some(value) => Some(value as &str),
          None => None,
        },
        hostname,
      ) && match &host.ip {
        Some(value) => ip_match(value as &str, ip),
        None => true,
      } {
        if let Some(new_data) = host.data.clone() {
          data = Some(new_data);
        }
        let mut error_config_used = false;
        if let Some(status_code) = status_code {
          for error_config in host.error_configs.iter() {
            if error_config.scode.is_none() || error_config.scode == Some(status_code) {
              if let Some(new_data) = error_config.data.clone() {
                data = Some(new_data);
              }
              error_config_used = true;
              break;
            }
          }
        }
        if !error_config_used {
          if let Ok(path_decoded) = urlencoding::decode(request_url) {
            for location in host.locations.iter() {
              if match_location(&location.path, &path_decoded) {
                if let Some(new_data) = location.data.clone() {
                  data = Some(new_data);
                }
                break;
              }
            }
          }
        }
        break;
      }
    }

    data
  }
}

impl<T> Clone for ObtainConfigStruct<T>
where
  T: Clone,
{
  fn clone(&self) -> Self {
    Self {
      global: self.global.clone(),
      host: self.host.clone(),
    }
  }
}
