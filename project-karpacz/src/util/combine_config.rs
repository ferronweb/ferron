use std::{net::IpAddr, sync::Arc};

use yaml_rust2::{yaml::Hash, Yaml};

use crate::project_karpacz_util::{ip_match::ip_match, match_hostname::match_hostname};

pub fn combine_config(
  config: Arc<Yaml>,
  hostname: Option<&str>,
  client_ip: IpAddr,
) -> Option<Yaml> {
  let global_config = config["global"].as_hash();
  let combined_config = global_config.cloned();

  if let Some(host_config) = config["hosts"].as_vec() {
    for host in host_config {
      if let Some(host_hashtable) = host.as_hash() {
        let domain_matched = host_hashtable
          .get(&Yaml::String("domain".to_string()))
          .and_then(Yaml::as_str)
          .map(|domain| match_hostname(Some(domain), hostname))
          .unwrap_or(true);

        let ip_matched = host_hashtable
          .get(&Yaml::String("ip".to_string()))
          .and_then(Yaml::as_str)
          .map(|ip| ip_match(ip, client_ip))
          .unwrap_or(true);

        if domain_matched && ip_matched {
          return Some(merge_configs(combined_config, host_hashtable));
        }
      }
    }
  }

  combined_config.map(Yaml::Hash)
}

fn merge_configs(global: Option<Hash>, host: &Hash) -> Yaml {
  let mut merged = global.unwrap_or_default();

  for (key, value) in host {
    match value {
      Yaml::Array(host_array) => {
        merged
          .entry(key.clone())
          .and_modify(|global_val| {
            if let Yaml::Array(global_array) = global_val {
              global_array.extend(host_array.clone());
            } else {
              *global_val = Yaml::Array(host_array.clone());
            }
          })
          .or_insert_with(|| Yaml::Array(host_array.clone()));
      }
      Yaml::Hash(host_hash) => {
        merged
          .entry(key.clone())
          .and_modify(|global_val| {
            if let Yaml::Hash(global_hash) = global_val {
              for (k, v) in host_hash {
                global_hash.insert(k.clone(), v.clone());
              }
            } else {
              *global_val = Yaml::Hash(host_hash.clone());
            }
          })
          .or_insert_with(|| Yaml::Hash(host_hash.clone()));
      }
      _ => {
        merged.insert(key.clone(), value.clone());
      }
    }
  }

  Yaml::Hash(merged)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::net::{IpAddr, Ipv4Addr};
  use yaml_rust2::{Yaml, YamlLoader};

  fn create_test_config() -> Arc<Yaml> {
    let yaml_str = r#"
        global:
          key1:
            - global_value1
          key2: 
            - global_value2
        hosts:
          - domain: example.com
            ip: 192.168.1.1
            key1: 
              - host_value1
            key2: 
              - host_value2
          - domain: test.com
            ip: 192.168.1.2
            key3: 
              - host_value3
        "#;

    let docs = YamlLoader::load_from_str(yaml_str).unwrap();
    Arc::new(docs[0].clone())
  }

  #[test]
  fn test_combine_config_with_matching_hostname_and_ip() {
    let config = create_test_config();
    let hostname = Some("example.com");
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

    let result = combine_config(config, hostname, client_ip);
    assert!(result.is_some());

    let result_yaml = result.unwrap();
    let result_hash = result_yaml.as_hash().unwrap();

    assert_eq!(
      result_hash
        .get(&Yaml::String("key1".to_string()))
        .unwrap()
        .as_vec()
        .unwrap()
        .len(),
      2
    );
    assert_eq!(
      result_hash
        .get(&Yaml::String("key2".to_string()))
        .unwrap()
        .as_vec()
        .unwrap()
        .len(),
      2
    );
  }

  #[test]
  fn test_combine_config_with_non_matching_hostname() {
    let config = create_test_config();
    let hostname = Some("nonexistent.com");
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

    let result = combine_config(config, hostname, client_ip);
    assert!(result
      .unwrap()
      .as_hash()
      .unwrap()
      .get(&Yaml::String(String::from("key3")))
      .is_none());
  }

  #[test]
  fn test_combine_config_with_non_matching_ip() {
    let config = create_test_config();
    let hostname = Some("example.com");
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));

    let result = combine_config(config, hostname, client_ip);
    assert!(result
      .unwrap()
      .as_hash()
      .unwrap()
      .get(&Yaml::String(String::from("key3")))
      .is_none());
  }

  #[test]
  fn test_combine_config_with_global_only() {
    let yaml_str = r#"
        global:
          key1: value1
          key2: 
            - global_value2
        hosts: []
        "#;

    let docs = YamlLoader::load_from_str(yaml_str).unwrap();
    let config = Arc::new(docs[0].clone());
    let hostname = None;
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

    let result = combine_config(config, hostname, client_ip);
    assert!(result.is_some());

    let result_yaml = result.unwrap();
    let result_hash = result_yaml.as_hash().unwrap();

    assert_eq!(
      result_hash
        .get(&Yaml::String("key1".to_string()))
        .unwrap()
        .as_str()
        .unwrap(),
      "value1"
    );
    assert_eq!(
      result_hash
        .get(&Yaml::String("key2".to_string()))
        .unwrap()
        .as_vec()
        .unwrap()
        .len(),
      1
    );
  }
}
