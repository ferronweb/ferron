pub mod adapters;
pub mod processing;

pub use ferron_common::config::*;

use std::fmt::Debug;
use std::net::IpAddr;
use std::sync::Arc;

use fancy_regex::RegexBuilder;

use crate::util::{is_localhost, match_hostname, match_location, replace_header_placeholders, IpBlockList};
use ferron_common::modules::SocketData;

/// Parses conditional data
pub fn parse_conditional_data(name: &str, value: ServerConfigurationEntry) -> ConditionalData {
  match name {
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
    "is_equal" => {
      if let Some(left_side) = value.values.first().and_then(|v| v.as_str()) {
        if let Some(right_side) = value.values.get(1).and_then(|v| v.as_str()) {
          ConditionalData::IsEqual(left_side.to_string(), right_side.to_string())
        } else {
          ConditionalData::Invalid
        }
      } else {
        ConditionalData::Invalid
      }
    }
    "is_not_equal" => {
      if let Some(left_side) = value.values.first().and_then(|v| v.as_str()) {
        if let Some(right_side) = value.values.get(1).and_then(|v| v.as_str()) {
          ConditionalData::IsNotEqual(left_side.to_string(), right_side.to_string())
        } else {
          ConditionalData::Invalid
        }
      } else {
        ConditionalData::Invalid
      }
    }
    "is_regex" => {
      if let Some(left_side) = value.values.first().and_then(|v| v.as_str()) {
        if let Some(right_side) = value.values.get(1).and_then(|v| v.as_str()).and_then(|v| {
          RegexBuilder::new(v)
            .case_insensitive(
              value
                .props
                .get("case_insensitive")
                .and_then(|p| p.as_bool())
                .unwrap_or(false),
            )
            .build()
            .ok()
        }) {
          ConditionalData::IsRegex(left_side.to_string(), right_side)
        } else {
          ConditionalData::Invalid
        }
      } else {
        ConditionalData::Invalid
      }
    }
    "is_not_regex" => {
      if let Some(left_side) = value.values.first().and_then(|v| v.as_str()) {
        if let Some(right_side) = value.values.get(1).and_then(|v| v.as_str()).and_then(|v| {
          RegexBuilder::new(v)
            .case_insensitive(
              value
                .props
                .get("case_insensitive")
                .and_then(|p| p.as_bool())
                .unwrap_or(false),
            )
            .build()
            .ok()
        }) {
          ConditionalData::IsNotRegex(left_side.to_string(), right_side)
        } else {
          ConditionalData::Invalid
        }
      } else {
        ConditionalData::Invalid
      }
    }
    _ => ConditionalData::Invalid,
  }
}

/// Matches conditions
fn match_conditions(conditions: &Conditions, request: &hyper::http::request::Parts, socket_data: &SocketData) -> bool {
  match_location(&conditions.location_prefix, request.uri.path())
    && conditions.conditionals.iter().all(|cond| match cond {
      Conditional::If(data) => data.iter().all(|d| match_condition(d, request, socket_data)),
      Conditional::IfNot(data) => !data.iter().all(|d| match_condition(d, request, socket_data)),
    })
}

/// Matches a condition
fn match_condition(
  condition: &ConditionalData,
  request: &hyper::http::request::Parts,
  socket_data: &SocketData,
) -> bool {
  match condition {
    ConditionalData::IsRemoteIp(list) => list.is_blocked(socket_data.remote_addr.ip()),
    ConditionalData::IsForwardedFor(list) => {
      let client_ip =
        if let Some(x_forwarded_for) = request.headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
          let prepared_remote_ip_str = match x_forwarded_for.split(",").next() {
            Some(ip_address_str) => ip_address_str.replace(" ", ""),
            None => return false,
          };

          let prepared_remote_ip: IpAddr = match prepared_remote_ip_str.parse() {
            Ok(ip_address) => ip_address,
            Err(_) => return false,
          };

          prepared_remote_ip
        } else {
          socket_data.remote_addr.ip()
        };

      list.is_blocked(client_ip)
    }
    ConditionalData::IsNotRemoteIp(list) => !list.is_blocked(socket_data.remote_addr.ip()),
    ConditionalData::IsNotForwardedFor(list) => {
      let client_ip =
        if let Some(x_forwarded_for) = request.headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
          let prepared_remote_ip_str = match x_forwarded_for.split(",").next() {
            Some(ip_address_str) => ip_address_str.replace(" ", ""),
            None => return false,
          };

          let prepared_remote_ip: IpAddr = match prepared_remote_ip_str.parse() {
            Ok(ip_address) => ip_address,
            Err(_) => return false,
          };

          prepared_remote_ip
        } else {
          socket_data.remote_addr.ip()
        };

      !list.is_blocked(client_ip)
    }
    ConditionalData::IsEqual(v1, v2) => {
      replace_header_placeholders(v1, request, Some(socket_data))
        == replace_header_placeholders(v2, request, Some(socket_data))
    }
    ConditionalData::IsNotEqual(v1, v2) => {
      replace_header_placeholders(v1, request, Some(socket_data))
        != replace_header_placeholders(v2, request, Some(socket_data))
    }
    ConditionalData::IsRegex(v1, regex) => regex
      .is_match(&replace_header_placeholders(v1, request, Some(socket_data)))
      .unwrap_or(false),
    ConditionalData::IsNotRegex(v1, regex) => {
      !(regex
        .is_match(&replace_header_placeholders(v1, request, Some(socket_data)))
        .unwrap_or(true))
    }
    _ => false,
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
  ) -> Option<Arc<ServerConfiguration>> {
    // The inner array is sorted by specifity, so it's easier to find the configurations.
    // If it was not sorted, we would need to implement the specifity...
    // Also, the approach mentioned in the line above might be slower...
    // But there is one thing we're wondering: so many logical operators???
    self
      .inner
      .iter()
      .rev()
      .find(|&server_configuration| {
        match_hostname(server_configuration.filters.hostname.as_deref(), hostname)
          && ((server_configuration.filters.ip.is_none() && (!is_localhost(server_configuration.filters.ip.as_ref(), server_configuration.filters.hostname.as_deref())
            || socket_data.local_addr.ip().to_canonical().is_loopback()))  // With special `localhost` check
            || server_configuration.filters.ip == Some(socket_data.local_addr.ip()))
          && (server_configuration.filters.port.is_none()
            || server_configuration.filters.port == Some(socket_data.local_addr.port()))
          && server_configuration
            .filters
            .condition
            .as_ref()
            .is_none_or(|c| match_conditions(c, request, socket_data))
          && server_configuration.filters.error_handler_status.is_none()
      })
      .cloned()
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
