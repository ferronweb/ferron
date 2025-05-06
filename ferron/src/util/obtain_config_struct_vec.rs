use std::error::Error;
use std::net::IpAddr;
use std::slice::Iter;
use std::sync::Arc;

use crate::ferron_common::ServerConfig;
use crate::ferron_util::ip_match::ip_match;
use crate::ferron_util::match_hostname::match_hostname;
use crate::ferron_util::match_location::match_location;

pub struct ObtainConfigStructVec<T> {
  global: Arc<Vec<T>>,
  host: Arc<Vec<ObtainConfigStructVecHost<T>>>,
}

struct ObtainConfigStructVecHost<T> {
  domain: Option<String>,
  ip: Option<String>,
  data: Vec<T>,
  locations: Vec<ObtainConfigStructVecLocation<T>>,
  error_configs: Vec<ObtainConfigStructVecErrorConfig<T>>,
}

struct ObtainConfigStructVecLocation<T> {
  path: String,
  data: Vec<T>,
}

struct ObtainConfigStructVecErrorConfig<T> {
  scode: Option<u16>,
  data: Vec<T>,
}

impl<T> ObtainConfigStructVec<T> {
  pub fn new(
    config: &ServerConfig,
    mut execute_fn: impl FnMut(&ServerConfig) -> Result<Vec<T>, Box<dyn Error + Send + Sync>>,
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
            error_configs.push(ObtainConfigStructVecErrorConfig {
              scode,
              data: execute_fn(error_config_yaml)?,
            });
          }
        }
        if let Some(locations_yaml) = host_yaml["locations"].as_vec() {
          for location_yaml in locations_yaml.iter() {
            if let Some(path_str) = location_yaml["path"].as_str() {
              let path = String::from(path_str);
              locations.push(ObtainConfigStructVecLocation {
                path,
                data: execute_fn(location_yaml)?,
              });
            }
          }
        }
        host_structs.push(ObtainConfigStructVecHost {
          domain,
          ip,
          data: execute_fn(host_yaml)?,
          locations,
          error_configs,
        });
      }
    }

    Ok(Self {
      global: Arc::new(global_struct),
      host: Arc::new(host_structs),
    })
  }
}

impl<'a, T> ObtainConfigStructVec<T>
where
  T: 'a,
{
  pub fn obtain(
    &'a self,
    hostname: Option<&str>,
    ip: IpAddr,
    request_url: &str,
    status_code: Option<u16>,
  ) -> Vec<&'a T> {
    let data_iter: Iter<'a, T> = self.global.iter();
    let mut host_data_iter: Box<dyn Iterator<Item = &'a T>> = Box::new(vec![].into_iter());
    let mut error_config_or_location_data_iter: Box<dyn Iterator<Item = &'a T>> =
      Box::new(vec![].into_iter());

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
        host_data_iter = Box::new(host.data.iter());
        let mut error_config_used = false;
        if let Some(status_code) = status_code {
          for location in host.error_configs.iter() {
            if location.scode.is_none() || location.scode == Some(status_code) {
              error_config_or_location_data_iter = Box::new(location.data.iter());
              error_config_used = true;
              break;
            }
          }
        }
        if !error_config_used {
          if let Ok(path_decoded) = urlencoding::decode(request_url) {
            for location in host.locations.iter() {
              if match_location(&location.path, &path_decoded) {
                error_config_or_location_data_iter = Box::new(location.data.iter());
                break;
              }
            }
          }
        }
        break;
      }
    }

    data_iter
      .chain(host_data_iter)
      .chain(error_config_or_location_data_iter)
      .collect()
  }
}

impl<T> Clone for ObtainConfigStructVec<T> {
  fn clone(&self) -> Self {
    Self {
      global: self.global.clone(),
      host: self.host.clone(),
    }
  }
}
