use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::sync::Arc;
use std::{collections::HashMap, net::IpAddr};

use ferron_common::util::{parse_q_value_header, replace_header_placeholders};
use ferron_common::{
  config::{Conditional, ConditionalData},
  modules::SocketData,
};

/// Condition match data
pub struct ConditionMatchData<'a> {
  pub request: &'a hyper::http::request::Parts,
  pub socket_data: &'a SocketData,
}

/// Matches conditional
pub fn match_conditional<'a>(
  conditional: &'a Conditional,
  match_data: &'a ConditionMatchData<'a>,
) -> Result<bool, Box<dyn Error + Send + Sync>> {
  let ConditionMatchData { request, socket_data } = match_data;
  if !(match conditional {
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
      let mut headers_hashmap_initial: HashMap<String, Vec<regorus::Value>> =
        HashMap::with_capacity(request.headers.keys_len());
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
    ConditionalData::IsLanguage(language) => {
      let accepted_languages = parse_q_value_header(
        request
          .headers
          .get(hyper::header::ACCEPT_LANGUAGE)
          .and_then(|v| v.to_str().ok())
          .unwrap_or("*"),
      );
      let supported_languages = constants
        .get("LANGUAGES")
        .and_then(|v| {
          let mut hash_set: HashSet<&str> = HashSet::new();
          for lang in v.split(",") {
            hash_set.insert(lang);
            if let Some((lang2, _)) = lang.split_once('-') {
              hash_set.insert(lang2);
            }
          }
          if hash_set.is_empty() {
            None
          } else {
            Some(hash_set)
          }
        })
        .unwrap_or(HashSet::from_iter(vec![language.as_str()]));
      Ok(
        accepted_languages
          .iter()
          .find(|l| {
            *l == "*"
              || supported_languages.contains(l.as_str())
              || l.split_once('-').is_some_and(|(v, _)| supported_languages.contains(v))
          })
          .is_some_and(|l| l == language || language.split_once('-').is_some_and(|(v, _)| v == l)),
      )
    }
    _ => Ok(false),
  }
}
