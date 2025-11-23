pub mod adapters;
pub mod processing;

pub use ferron_common::config::*;

use std::collections::HashMap;
use std::error::Error;
use std::net::IpAddr;
use std::sync::Arc;
use std::{collections::BTreeMap, fmt::Debug};

use fancy_regex::RegexBuilder;

use crate::util::{is_localhost, match_hostname, match_location, replace_header_placeholders, IpBlockList};
use ferron_common::modules::SocketData;

/// Parses conditional data
pub fn parse_conditional_data(
  name: &str,
  value: ServerConfigurationEntry,
) -> Result<ConditionalData, Box<dyn Error + Send + Sync>> {
  Ok(match name {
    "is_remote_ip" => {
      let mut list = IpBlockList::new();
      list.load_from_vec(value.values.iter().filter_map(|v| v.as_str()).collect());
      ConditionalData::IsRemoteIp(list)
    }
    "is_forwarded_for" => {
      let mut list = IpBlockList::new();
      list.load_from_vec(value.values.iter().filter_map(|v| v.as_str()).collect());
      ConditionalData::IsForwardedFor(list)
    }
    "is_not_remote_ip" => {
      let mut list = IpBlockList::new();
      list.load_from_vec(value.values.iter().filter_map(|v| v.as_str()).collect());
      ConditionalData::IsNotRemoteIp(list)
    }
    "is_not_forwarded_for" => {
      let mut list = IpBlockList::new();
      list.load_from_vec(value.values.iter().filter_map(|v| v.as_str()).collect());
      ConditionalData::IsNotForwardedFor(list)
    }
    "is_equal" => ConditionalData::IsEqual(
      value
        .values
        .first()
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid left side of a \"is_equal\" subcondition"
        ))?
        .to_string(),
      value
        .values
        .get(1)
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid right side of a \"is_equal\" subcondition"
        ))?
        .to_string(),
    ),
    "is_not_equal" => ConditionalData::IsNotEqual(
      value
        .values
        .first()
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid left side of a \"is_not_equal\" subcondition"
        ))?
        .to_string(),
      value
        .values
        .get(1)
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid right side of a \"is_not_equal\" subcondition"
        ))?
        .to_string(),
    ),
    "is_regex" => {
      let left_side = value.values.first().and_then(|v| v.as_str()).ok_or(anyhow::anyhow!(
        "Missing or invalid left side of a \"is_regex\" subcondition"
      ))?;
      let right_side = value.values.get(1).and_then(|v| v.as_str()).ok_or(anyhow::anyhow!(
        "Missing or invalid right side of a \"is_regex\" subcondition"
      ))?;
      ConditionalData::IsRegex(
        left_side.to_string(),
        RegexBuilder::new(right_side)
          .case_insensitive(
            value
              .props
              .get("case_insensitive")
              .and_then(|p| p.as_bool())
              .unwrap_or(false),
          )
          .build()?,
      )
    }
    "is_not_regex" => {
      let left_side = value.values.first().and_then(|v| v.as_str()).ok_or(anyhow::anyhow!(
        "Missing or invalid left side of a \"is_not_regex\" subcondition"
      ))?;
      let right_side = value.values.get(1).and_then(|v| v.as_str()).ok_or(anyhow::anyhow!(
        "Missing or invalid right side of a \"is_not_regex\" subcondition"
      ))?;
      ConditionalData::IsNotRegex(
        left_side.to_string(),
        RegexBuilder::new(right_side)
          .case_insensitive(
            value
              .props
              .get("case_insensitive")
              .and_then(|p| p.as_bool())
              .unwrap_or(false),
          )
          .build()?,
      )
    }
    "is_rego" => {
      let rego_policy = value
        .values
        .first()
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!("Missing or invalid Rego policy"))?;
      let mut rego_engine = regorus::Engine::new();
      rego_engine.add_policy("ferron.rego".to_string(), rego_policy.to_string())?;
      ConditionalData::IsRego(Arc::new(rego_engine))
    }
    "set_constant" => ConditionalData::SetConstant(
      value
        .values
        .first()
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid constant name in a \"set_constant\" subcondition"
        ))?
        .to_string(),
      value
        .values
        .get(1)
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid constant value in a \"set_constant\" subcondition"
        ))?
        .to_string(),
    ),
    _ => Err(anyhow::anyhow!("Unrecognized subcondition: {name}"))?,
  })
}

/// Matches conditions
fn match_conditions(
  conditions: &Conditions,
  request: &hyper::http::request::Parts,
  socket_data: &SocketData,
) -> Result<bool, Box<dyn Error + Send + Sync>> {
  if !match_location(&conditions.location_prefix, request.uri.path()) {
    return Ok(false);
  }
  for cond in &conditions.conditionals {
    if !(match cond {
      Conditional::If(data) => {
        let mut matches = true;
        let mut constants = HashMap::new();
        for d in data {
          if !match_condition(d, request, socket_data, &mut constants)? {
            matches = false;
            break;
          }
        }
        matches
      }
      Conditional::IfNot(data) => {
        let mut matches = true;
        let mut constants = HashMap::new();
        for d in data {
          if !match_condition(d, request, socket_data, &mut constants)? {
            matches = false;
            break;
          }
        }
        !matches
      }
    }) {
      return Ok(false);
    }
  }
  Ok(true)
}

/// Matches a condition
fn match_condition(
  condition: &ConditionalData,
  request: &hyper::http::request::Parts,
  socket_data: &SocketData,
  constants: &mut HashMap<String, String>,
) -> Result<bool, Box<dyn Error + Send + Sync>> {
  match condition {
    ConditionalData::IsRemoteIp(list) => Ok(list.is_blocked(socket_data.remote_addr.ip())),
    ConditionalData::IsForwardedFor(list) => {
      let client_ip =
        if let Some(x_forwarded_for) = request.headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
          let prepared_remote_ip_str = match x_forwarded_for.split(",").next() {
            Some(ip_address_str) => ip_address_str.replace(" ", ""),
            None => return Ok(false),
          };

          let prepared_remote_ip: IpAddr = match prepared_remote_ip_str.parse() {
            Ok(ip_address) => ip_address,
            Err(_) => return Ok(false),
          };

          prepared_remote_ip
        } else {
          socket_data.remote_addr.ip()
        };

      Ok(list.is_blocked(client_ip))
    }
    ConditionalData::IsNotRemoteIp(list) => Ok(!list.is_blocked(socket_data.remote_addr.ip())),
    ConditionalData::IsNotForwardedFor(list) => {
      let client_ip =
        if let Some(x_forwarded_for) = request.headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
          let prepared_remote_ip_str = match x_forwarded_for.split(",").next() {
            Some(ip_address_str) => ip_address_str.replace(" ", ""),
            None => return Ok(false),
          };

          let prepared_remote_ip: IpAddr = match prepared_remote_ip_str.parse() {
            Ok(ip_address) => ip_address,
            Err(_) => return Ok(false),
          };

          prepared_remote_ip
        } else {
          socket_data.remote_addr.ip()
        };

      Ok(!list.is_blocked(client_ip))
    }
    ConditionalData::IsEqual(v1, v2) => Ok(
      replace_header_placeholders(v1, request, Some(socket_data))
        == replace_header_placeholders(v2, request, Some(socket_data)),
    ),
    ConditionalData::IsNotEqual(v1, v2) => Ok(
      replace_header_placeholders(v1, request, Some(socket_data))
        != replace_header_placeholders(v2, request, Some(socket_data)),
    ),
    ConditionalData::IsRegex(v1, regex) => {
      Ok(regex.is_match(&replace_header_placeholders(v1, request, Some(socket_data)))?)
    }
    ConditionalData::IsNotRegex(v1, regex) => {
      Ok(!(regex.is_match(&replace_header_placeholders(v1, request, Some(socket_data)))?))
    }
    ConditionalData::IsRego(rego_engine) => {
      let mut cloned_engine = (*rego_engine.clone()).clone();
      let mut rego_input_object = BTreeMap::new();
      rego_input_object.insert("method".into(), request.method.as_str().into());
      rego_input_object.insert(
        "protocol".into(),
        match request.version {
          hyper::Version::HTTP_09 => "HTTP/0.9".into(),
          hyper::Version::HTTP_10 => "HTTP/1.0".into(),
          hyper::Version::HTTP_11 => "HTTP/1.1".into(),
          hyper::Version::HTTP_2 => "HTTP/2.0".into(),
          hyper::Version::HTTP_3 => "HTTP/3.0".into(),
          _ => "HTTP/Unknown".into(),
        },
      );
      rego_input_object.insert("uri".into(), request.uri.to_string().into());
      let mut headers_hashmap_initial: HashMap<String, Vec<regorus::Value>> = HashMap::new();
      for (key, value) in request.headers.iter() {
        let key_string = key.as_str().to_lowercase();
        if let Some(header_list) = headers_hashmap_initial.get_mut(&key_string) {
          header_list.push(value.to_str().unwrap_or("").into());
        } else {
          headers_hashmap_initial.insert(key_string, vec![value.to_str().unwrap_or("").into()]);
        }
      }
      let mut headers_btreemap = BTreeMap::new();
      for (key, value) in headers_hashmap_initial.into_iter() {
        headers_btreemap.insert(key.into(), value.into());
      }
      let headers_rego = regorus::Value::Object(Arc::new(headers_btreemap));
      rego_input_object.insert("headers".into(), headers_rego);
      let mut socket_data_btreemap = BTreeMap::new();
      socket_data_btreemap.insert("client_ip".into(), socket_data.remote_addr.ip().to_string().into());
      socket_data_btreemap.insert("client_port".into(), (socket_data.remote_addr.port() as u32).into());
      socket_data_btreemap.insert("server_ip".into(), socket_data.local_addr.ip().to_string().into());
      socket_data_btreemap.insert("server_port".into(), (socket_data.local_addr.port() as u32).into());
      socket_data_btreemap.insert("encrypted".into(), socket_data.encrypted.into());
      let socket_data_rego = regorus::Value::Object(Arc::new(socket_data_btreemap));
      rego_input_object.insert("socket_data".into(), socket_data_rego);
      let mut constants_btreemap = BTreeMap::new();
      for (key, value) in constants.iter_mut() {
        constants_btreemap.insert(key.to_owned().into(), value.to_owned().into());
      }
      let constants_rego = regorus::Value::Object(Arc::new(constants_btreemap));
      rego_input_object.insert("constants".into(), constants_rego);
      let rego_input = regorus::Value::Object(Arc::new(rego_input_object));
      cloned_engine.set_input(rego_input);
      Ok(*cloned_engine.eval_rule("data.ferron.pass".to_string())?.as_bool()?)
    }
    ConditionalData::SetConstant(name, value) => {
      constants.insert(name.to_owned(), value.to_owned());
      Ok(true)
    }
    _ => Ok(false),
  }
}

/// The struct containing all the Ferron server configurations
#[derive(Debug)]
pub struct ServerConfigurations {
  /// Vector of configurations
  pub inner: Vec<Arc<ServerConfiguration>>,
}

impl ServerConfigurations {
  /// Creates the server configurations struct
  pub fn new(mut inner: Vec<ServerConfiguration>) -> Self {
    // Reverse the inner vector to ensure the location configurations are checked in the correct order
    inner.reverse();

    // Sort the configurations array by the specifity of configurations, so that it will be possible to find the configuration
    inner.sort_by(|a, b| a.filters.cmp(&b.filters));
    Self {
      inner: inner.into_iter().map(Arc::new).collect(),
    }
  }

  /// Finds a specific server configuration based on request parameters
  pub fn find_configuration(
    &self,
    request: &hyper::http::request::Parts,
    hostname: Option<&str>,
    socket_data: &SocketData,
  ) -> Result<Option<Arc<ServerConfiguration>>, Box<dyn Error + Send + Sync>> {
    // The inner array is sorted by specifity, so it's easier to find the configurations.
    // If it was not sorted, we would need to implement the specifity...
    // Also, the approach mentioned in the line above might be slower...
    // But there is one thing we're wondering: so many logical operators???
    for server_configuration in self.inner.iter().rev() {
      if match_hostname(server_configuration.filters.hostname.as_deref(), hostname)
        && ((server_configuration.filters.ip.is_none() && (!is_localhost(server_configuration.filters.ip.as_ref(), server_configuration.filters.hostname.as_deref())
            || socket_data.local_addr.ip().to_canonical().is_loopback()))  // With special `localhost` check
            || server_configuration.filters.ip == Some(socket_data.local_addr.ip()))
        && (server_configuration.filters.port.is_none()
          || server_configuration.filters.port == Some(socket_data.local_addr.port()))
        && server_configuration
          .filters
          .condition
          .as_ref()
          .map(|c| match_conditions(c, request, socket_data))
          .unwrap_or(Ok(true))?
        && server_configuration.filters.error_handler_status.is_none()
      {
        return Ok(Some(server_configuration.clone()));
      }
    }

    Ok(None)
  }

  /// Finds the server error configuration based on configuration filters
  pub fn find_error_configuration(
    &self,
    filters: &ServerConfigurationFilters,
    status_code: u16,
  ) -> Option<Arc<ServerConfiguration>> {
    self
      .inner
      .iter()
      .rev()
      .find(|c| {
        c.filters.is_host
          && c.filters.hostname == filters.hostname
          && c.filters.ip == filters.ip
          && c.filters.port == filters.port
          && (c.filters.condition.is_none() || c.filters.condition == filters.condition)
          && !c.filters.error_handler_status.as_ref().is_none_or(|s| {
            !(matches!(s, ErrorHandlerStatus::Any) || matches!(s, ErrorHandlerStatus::Status(x) if *x == status_code))
          })
      })
      .cloned()
  }

  /// Finds the global server configuration (host or non-host)
  pub fn find_global_configuration(&self) -> Option<Arc<ServerConfiguration>> {
    // The server configurations are pre-merged, so we can simply return the found global configuration
    let mut iterator = self.inner.iter();
    let first_found = iterator.find(|server_configuration| {
      server_configuration.filters.is_global() || server_configuration.filters.is_global_non_host()
    });
    if let Some(first_found) = first_found {
      if first_found.filters.is_global() {
        return Some(first_found.clone());
      }
      for server_configuration in iterator {
        if server_configuration.filters.is_global() {
          return Some(server_configuration.clone());
        } else if !server_configuration.filters.is_global_non_host() {
          return Some(first_found.clone());
        }
      }
    }
    None
  }
}
