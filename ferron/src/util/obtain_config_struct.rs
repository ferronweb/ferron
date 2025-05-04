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
}

struct ObtainConfigStructLocation<T> {
  path: String,
  data: T,
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
        let mut locations = Vec::new();
        if let Some(locations_yaml) = host_yaml["locations"].as_vec() {
          for location_yaml in locations_yaml.iter() {
            if let Some(path_str) = location_yaml["path"].as_str() {
              let path = String::from(path_str);
              if let Some(data) = execute_fn(location_yaml)? {
                locations.push(ObtainConfigStructLocation { path, data });
              }
            }
          }
        }
        host_structs.push(ObtainConfigStructHost {
          domain,
          ip,
          data: execute_fn(host_yaml)?,
          locations,
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
  pub fn obtain(&mut self, hostname: Option<&str>, ip: IpAddr, request_url: &str) -> Option<T> {
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
        data = host.data.clone();
        if let Ok(path_decoded) = urlencoding::decode(request_url) {
          for location in host.locations.iter() {
            if match_location(&location.path, &path_decoded) {
              data = Some(location.data.clone());
              break;
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
