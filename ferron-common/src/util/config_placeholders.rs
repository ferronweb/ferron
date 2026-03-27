use std::env;
use anyhow::{bail, Result};

fn resolve_placeholder(kind: &str, value: &str) -> Result<Option<String>> {
    match kind {
        "env" => match env::var(value) {
            Ok(val) => Ok(Some(val)),
            Err(e) => bail!("couldn't find ENV-Key >>{}<< {}", value, e),
        },
        _ => Ok(None),
    }
}

pub fn replace_placeholders(input: &str) -> Result<String> {
    let mut output = String::new();
    let mut cursor = 0;

    loop {
        let start_rel = match input[cursor..].find('{') {
            Some(pos) => pos,
            None => {
                output.push_str(&input[cursor..]);
                break;
            }
        };

        let start = cursor + start_rel;

        let end_rel = match input[start + 1..].find('}') {
            Some(pos) => pos,
            None => bail!("No closing '}}' found for '{{' at position {}", start),
        };

        let end = start + 1 + end_rel;

        // Push preceding text
        output.push_str(&input[cursor..start]);

        let placeholder = &input[start + 1..end];

        // Split into kind:value
        if let Some((kind, value)) = placeholder.split_once(':') {
            match resolve_placeholder(kind, value)? {
                Some(resolved) => output.push_str(&resolved),
                None => {
                    // Unknown kind → keep original
                    output.push('{');
                    output.push_str(placeholder);
                    output.push('}');
                }
            }
        } else {
            // No ':' → not a structured placeholder, keep as-is
            output.push('{');
            output.push_str(placeholder);
            output.push('}');
        }

        cursor = end + 1;
    }

    Ok(output)
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
    let result = lookup_config_placeholders("LALA".to_string());
    assert!(result.is_ok(), "Strings without env markers should pass through");
    assert_eq!(result.unwrap(), "LALA");
  }

  #[test]
  fn lookup_config_placeholders() {
    let result = lookup_config_placeholders("{env:HOME}".to_string());
    assert!(result.is_ok(), "Get successfully 'HOME' ENV");
  }
  #[test]
  fn lookup_config_placeholders() {
    // {envA:HOME} doesn't contain {env: so it passes through unchanged
    let result = lookup_config_placeholders("{envA:HOME}".to_string());
    assert!(
      result.is_ok(),
      "Strings without proper {{env: markers should pass through"
    );
    assert_eq!(result.unwrap(), "{envA:HOME}");
  }

  #[test]
  fn interpolate_env_value() {
    let result = lookup_config_placeholders("{env:HOME}/src/modules".to_string());
    assert!(result.is_ok(), "Successfully interpolated HOME with path suffix");
    let value = result.unwrap();
    assert!(value.ends_with("/src/modules"), "Should end with /src/modules");
    assert!(!value.contains("{env:"), "Should not contain {{env: marker");
  }

  #[test]
  fn interpolate_multiple_env_values() {
    let result = lookup_config_placeholders("prefix_{env:HOME}_middle_{env:USER}_suffix".to_string());
    assert!(result.is_ok(), "Successfully interpolated multiple env vars");
    let value = result.unwrap();
    assert!(value.starts_with("prefix_"), "Should start with prefix_");
    assert!(value.ends_with("_suffix"), "Should end with _suffix");
    assert!(!value.contains("{env:"), "Should not contain {{env: marker");
  }

  #[test]
  fn no_env_passthrough() {
    let result = lookup_config_placeholders("plain_string_without_env".to_string());
    assert!(result.is_ok(), "Plain strings should pass through");
    assert_eq!(result.unwrap(), "plain_string_without_env");
  }

  #[test]
  fn missing_closing_brace() {
    let result = lookup_config_placeholders("{env:HOME".to_string());
    assert!(result.is_err(), "Should error on missing closing brace");
  }

  #[test]
  fn nonexistent_env_var() {
    let result = lookup_config_placeholders("{env:NONEXISTENT_VAR_THAT_SHOULD_NOT_EXIST}".to_string());
    assert!(result.is_err(), "Should error when env var doesn't exist");
  }

  #[test]
  fn nonexistent_env_var_in_interpolation() {
    let result = lookup_config_placeholders("prefix_{env:NONEXISTENT_VAR}_suffix".to_string());
    assert!(result.is_err(), "Should error when interpolated env var doesn't exist");
  }
}
