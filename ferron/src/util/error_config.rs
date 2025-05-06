use yaml_rust2::Yaml;

fn get_error_config(config: &Yaml, status_code: u16) -> Option<Yaml> {
  if let Some(error_configs) = config["errorConfig"].as_vec() {
    for error_config in error_configs {
      let configured_status_code = error_config["scode"].as_i64();
      if configured_status_code.is_none() || configured_status_code == Some(status_code as i64) {
        return Some(error_config.to_owned());
      }
    }
  }
  None
}

pub fn combine_error_config(config: &Yaml, status_code: u16) -> Option<Yaml> {
  if let Some(error_config) = get_error_config(config, status_code) {
    if let Some(error_config_hash) = error_config.as_hash() {
      if let Some(config_hash) = config.as_hash() {
        let mut merged = config_hash.clone();
        while merged.remove(&Yaml::from_str("errorConfig")).is_some() {}
        for (key, value) in error_config_hash {
          if let Some(key) = key.as_str() {
            if key != "errorConfig" && key != "scode" {
              match value {
                Yaml::Array(host_array) => {
                  merged
                    .entry(Yaml::String(key.to_string()))
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
                    .entry(Yaml::String(key.to_string()))
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
                  merged.insert(Yaml::String(key.to_string()), value.clone());
                }
              }
            }
          }
        }
        return Some(Yaml::Hash(merged));
      }
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use yaml_rust2::{Yaml, YamlLoader};

  use super::*;

  fn load_yaml(input: &str) -> Yaml {
    YamlLoader::load_from_str(input).unwrap()[0].clone()
  }

  #[test]
  fn test_no_error_config() {
    let yaml = load_yaml(
      r#"
            name: AppConfig
            version: 1
        "#,
    );
    assert_eq!(combine_error_config(&yaml, 404), None);
  }

  #[test]
  fn test_no_matching_status_code() {
    let yaml = load_yaml(
      r#"
            errorConfig:
              - scode: 500
                message: "Internal Server Error"
        "#,
    );
    assert_eq!(combine_error_config(&yaml, 404), None);
  }

  #[test]
  fn test_catch_all_error_config() {
    let yaml = load_yaml(
      r#"
            errorConfig:
              - message: "An error occurred"
        "#,
    );
    let result = combine_error_config(&yaml, 500).unwrap();
    assert_eq!(result["message"].as_str().unwrap(), "An error occurred");
  }

  #[test]
  fn test_simple_merge() {
    let yaml = load_yaml(
      r#"
            name: AppConfig
            errorConfig:
              - scode: 404
                message: "Not Found"
        "#,
    );
    let result = combine_error_config(&yaml, 404).unwrap();
    assert_eq!(result["message"].as_str().unwrap(), "Not Found");
    assert_eq!(result["name"].as_str().unwrap(), "AppConfig");
  }

  #[test]
  fn test_merge_array() {
    let yaml = load_yaml(
      r#"
            paths: ["/home"]
            errorConfig:
              - scode: 404
                paths: ["/error"]
        "#,
    );
    let result = combine_error_config(&yaml, 404).unwrap();
    let paths = result["paths"].as_vec().unwrap();
    let path_strs: Vec<_> = paths.iter().map(|p| p.as_str().unwrap()).collect();
    assert_eq!(path_strs, vec!["/home", "/error"]);
  }

  #[test]
  fn test_merge_hash() {
    let yaml = load_yaml(
      r#"
            settings:
              theme: "light"
            errorConfig:
              - scode: 404
                settings:
                  debug: true
        "#,
    );
    let result = combine_error_config(&yaml, 404).unwrap();
    let settings = result["settings"].as_hash().unwrap();
    assert_eq!(
      settings
        .get(&Yaml::String("theme".into()))
        .unwrap()
        .as_str()
        .unwrap(),
      "light"
    );
    assert!(settings
      .get(&Yaml::String("debug".into()))
      .unwrap()
      .as_bool()
      .unwrap());
  }

  #[test]
  fn test_override_non_array_with_array() {
    let yaml = load_yaml(
      r#"
            value: 42
            errorConfig:
              - scode: 404
                value: [1, 2, 3]
        "#,
    );
    let result = combine_error_config(&yaml, 404).unwrap();
    let arr = result["value"].as_vec().unwrap();
    let values: Vec<_> = arr.iter().map(|v| v.as_i64().unwrap()).collect();
    assert_eq!(values, vec![1, 2, 3]);
  }

  #[test]
  fn test_override_non_hash_with_hash() {
    let yaml = load_yaml(
      r#"
            meta: "info"
            errorConfig:
              - scode: 404
                meta:
                  tag: "404"
        "#,
    );
    let result = combine_error_config(&yaml, 404).unwrap();
    let meta = result["meta"].as_hash().unwrap();
    assert_eq!(
      meta
        .get(&Yaml::String("tag".into()))
        .unwrap()
        .as_str()
        .unwrap(),
      "404"
    );
  }

  #[test]
  fn test_ignore_error_config_key_in_merge() {
    let yaml = load_yaml(
      r#"
            name: App
            errorConfig:
              - scode: 404
                errorConfig: "should be ignored"
                message: "Not Found"
        "#,
    );
    let result = combine_error_config(&yaml, 404).unwrap();
    assert!(result["errorConfig"] != Yaml::String("should be ignored".to_string())); // still an array or ignored
    assert_eq!(result["message"].as_str().unwrap(), "Not Found");
  }
}
