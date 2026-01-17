use anyhow::bail;
use std::env;

pub fn lookup_env_value(conf_string: String) -> anyhow::Result<String> {
  if !conf_string.contains("{env:") {
    return Ok(conf_string);
  }

  let mut result = String::new();
  let mut last_end = 0;

  for (start, _) in conf_string.match_indices("{env:") {
    // Add everything before this match
    result.push_str(&conf_string[last_end..start]);

    // Find the closing brace
    let search_from = start + 5;
    if let Some(end_offset) = conf_string[search_from..].find('}') {
      let end = search_from + end_offset;
      let env_var_name = &conf_string[search_from..end];

      // Get and append the env value
      let env_value = get_env_val(env_var_name.to_string())?;
      result.push_str(&env_value);

      last_end = end + 1;
    } else {
      bail!("No closing '}}' found for '{{env:' at position {}", start);
    }
  }

  // Add any remaining text
  result.push_str(&conf_string[last_end..]);

  Ok(result)
}

pub fn get_env_val(conf_key: String) -> anyhow::Result<String> {
  match env::var(conf_key.clone()) {
    Ok(val) => Ok(val),
    Err(e) => bail!("couldn't find ENV-Key >>{conf_key}<< {e}"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn lala_get_env_var() {
    let result = get_env_val("LALA".to_string());
    assert!(
      result.is_err(),
      "couldn't find ENV-Key >>LALA<< environment variable not found"
    );
  }

  #[test]
  fn home_get_env_var() {
    let result = get_env_val("HOME".to_string());
    assert!(result.is_ok(), "Found 'HOME'");
  }

  #[test]
  fn missing_get_env_key() {
    // Strings without {env:} are now passed through unchanged
    let result = lookup_env_value("LALA".to_string());
    assert!(result.is_ok(), "Strings without env markers should pass through");
    assert_eq!(result.unwrap(), "LALA");
  }

  #[test]
  fn success_lookup_env_value() {
    let result = lookup_env_value("{env:HOME}".to_string());
    assert!(result.is_ok(), "Get successfully 'HOME' ENV");
  }
  #[test]
  fn failed_lookup_env_value() {
    // {envA:HOME} doesn't contain {env: so it passes through unchanged
    let result = lookup_env_value("{envA:HOME}".to_string());
    assert!(
      result.is_ok(),
      "Strings without proper {{env: markers should pass through"
    );
    assert_eq!(result.unwrap(), "{envA:HOME}");
  }

  #[test]
  fn interpolate_env_value() {
    let result = lookup_env_value("{env:HOME}/src/modules".to_string());
    assert!(result.is_ok(), "Successfully interpolated HOME with path suffix");
    let value = result.unwrap();
    assert!(value.ends_with("/src/modules"), "Should end with /src/modules");
    assert!(!value.contains("{env:"), "Should not contain {{env: marker");
  }

  #[test]
  fn interpolate_multiple_env_values() {
    let result = lookup_env_value("prefix_{env:HOME}_middle_{env:USER}_suffix".to_string());
    assert!(result.is_ok(), "Successfully interpolated multiple env vars");
    let value = result.unwrap();
    assert!(value.starts_with("prefix_"), "Should start with prefix_");
    assert!(value.ends_with("_suffix"), "Should end with _suffix");
    assert!(!value.contains("{env:"), "Should not contain {{env: marker");
  }

  #[test]
  fn no_env_passthrough() {
    let result = lookup_env_value("plain_string_without_env".to_string());
    assert!(result.is_ok(), "Plain strings should pass through");
    assert_eq!(result.unwrap(), "plain_string_without_env");
  }

  #[test]
  fn missing_closing_brace() {
    let result = lookup_env_value("{env:HOME".to_string());
    assert!(result.is_err(), "Should error on missing closing brace");
  }

  #[test]
  fn nonexistent_env_var() {
    let result = lookup_env_value("{env:NONEXISTENT_VAR_THAT_SHOULD_NOT_EXIST}".to_string());
    assert!(result.is_err(), "Should error when env var doesn't exist");
  }

  #[test]
  fn nonexistent_env_var_in_interpolation() {
    let result = lookup_env_value("prefix_{env:NONEXISTENT_VAR}_suffix".to_string());
    assert!(result.is_err(), "Should error when interpolated env var doesn't exist");
  }
}
