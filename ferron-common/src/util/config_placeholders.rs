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
    use std::env;

    #[test]
    fn env_var_missing() {
        let result = resolve_placeholder("env", "LALA_SHOULD_NOT_EXIST");
        assert!(result.is_err());
    }

    #[test]
    fn env_var_exists() {
        let result = resolve_placeholder("env", "HOME");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn passthrough_no_placeholders() {
        let input = "LALA";
        let result = replace_placeholders(input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), input);
    }

    #[test]
    fn single_env_placeholder() {
        let result = replace_placeholders("{env:HOME}");
        assert!(result.is_ok());
        let value = result.unwrap();
        assert!(!value.contains("{env:"));
    }

    #[test]
    fn unknown_kind_passthrough() {
        let result = replace_placeholders("{envA:HOME}");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "{envA:HOME}");
    }

    #[test]
    fn interpolate_env_with_suffix() {
        let result = replace_placeholders("{env:HOME}/src/modules");
        assert!(result.is_ok());
        let value = result.unwrap();
        assert!(value.ends_with("/src/modules"));
        assert!(!value.contains("{env:"));
    }

    #[test]
    fn interpolate_multiple_env_values() {
        std::env::set_var("TEST_HOME", "/home/test");
        std::env::set_var("TEST_USER", "user");

        let input = "prefix_{env:TEST_HOME}_middle_{env:TEST_USER}_suffix";
        let result = replace_placeholders(input).unwrap();

        let expected = "prefix_/home/test_middle_user_suffix";
        assert_eq!(result, expected);
    }

    

    #[test]
    fn plain_string_passthrough() {
        let input = "plain_string_without_env";
        let result = replace_placeholders(input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), input);
    }

    #[test]
    fn missing_closing_brace() {
        let result = replace_placeholders("{env:HOME");
        assert!(result.is_err());
    }

    #[test]
    fn nonexistent_env_var() {
        let result =
            replace_placeholders("{env:NONEXISTENT_VAR_THAT_SHOULD_NOT_EXIST}");
        assert!(result.is_err());
    }

    #[test]
    fn nonexistent_env_var_in_interpolation() {
        let result =
            replace_placeholders("prefix_{env:NONEXISTENT_VAR}_suffix");
        assert!(result.is_err());
    }

    #[test]
    fn placeholder_without_colon_passthrough() {
        let result = replace_placeholders("{justtext}");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "{justtext}");
    }
}
