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

    while cursor < input.len() {
        let next = input[cursor..].find('{');

        let start_rel = match next {
            Some(pos) => pos,
            None => {
                output.push_str(&input[cursor..]);
                break;
            }
        };

        let start = cursor + start_rel;

        // Check if escaped: "\{"
        if start > 0 && input.as_bytes()[start - 1] == b'\\' {
            // push everything before the backslash
            output.push_str(&input[cursor..start - 1]);
            // push literal '{'
            output.push('{');

            cursor = start + 1;
            continue;
        }

        let end_rel = match input[start + 1..].find('}') {
            Some(pos) => pos,
            None => bail!("No closing '}}' found for '{{' at position {}", start),
        };

        let end = start + 1 + end_rel;

        // Push preceding text
        output.push_str(&input[cursor..start]);

        let placeholder = &input[start + 1..end];

        if let Some((kind, value)) = placeholder.split_once(':') {
            match resolve_placeholder(kind, value)? {
                Some(resolved) => output.push_str(&resolved),
                None => {
                    output.push('{');
                    output.push_str(placeholder);
                    output.push('}');
                }
            }
        } else {
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
        env::set_var("TEST_ENV_EXISTS", "value");

        let result = resolve_placeholder("env", "TEST_ENV_EXISTS");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some("value".to_string()));
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
        env::set_var("TEST_HOME", "/home/test");

        let result = replace_placeholders("{env:TEST_HOME}");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "/home/test");
    }

    #[test]
    fn unknown_kind_passthrough() {
        let result = replace_placeholders("{envA:HOME}");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "{envA:HOME}");
    }

    #[test]
    fn interpolate_env_with_suffix() {
        env::set_var("TEST_HOME", "/home/test");

        let result = replace_placeholders("{env:TEST_HOME}/src/modules");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "/home/test/src/modules");
    }

    #[test]
    fn interpolate_multiple_env_values() {
        env::set_var("TEST_HOME", "/home/test");
        env::set_var("TEST_USER", "user");

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
        let result = replace_placeholders("{env:TEST_HOME");
        assert!(result.is_err());
    }

    #[test]
    fn nonexistent_env_var() {
        let result =
            replace_placeholders("{env:THIS_SHOULD_NOT_EXIST_123}");
        assert!(result.is_err());
    }

    #[test]
    fn nonexistent_env_var_in_interpolation() {
        let result =
            replace_placeholders("prefix_{env:THIS_SHOULD_NOT_EXIST_456}_suffix");
        assert!(result.is_err());
    }

    #[test]
    fn placeholder_without_colon_passthrough() {
        let result = replace_placeholders("{justtext}");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "{justtext}");
    }

    #[test]
    fn escaped_open_brace() {
        // No env needed — should NOT resolve
        let result = replace_placeholders(r"\{env:TEST_HOME}").unwrap();
        assert_eq!(result, "{env:TEST_HOME}");
    }

    #[test]
    fn escaped_brace_with_text() {
        let result = replace_placeholders(r"prefix_\{env:TEST_HOME}_suffix").unwrap();
        assert_eq!(result, "prefix_{env:TEST_HOME}_suffix");
    }

    #[test]
    fn escaped_and_real_placeholder() {
        std::env::set_var("TEST_HOME_ESC", "/home/test");

        let input = r"\{env:TEST_HOME_ESC}_{env:TEST_HOME_ESC}";
        let result = replace_placeholders(input).unwrap();

        assert_eq!(result, "{env:TEST_HOME_ESC}_/home/test");
    }

    #[test]
    fn double_escape_sequence() {
        std::env::set_var("TEST_HOME_ESC2", "/home/test");

        // "\\{" → literal "\" + "{"
        let result = replace_placeholders(r"\\{env:TEST_HOME_ESC2}").unwrap();

        // First "\" is literal, then placeholder is evaluated
        assert_eq!(result, r"\/home/test");
    }

    #[test]
    fn escaped_non_placeholder() {
        let result = replace_placeholders(r"\{justtext}").unwrap();
        assert_eq!(result, "{justtext}");
    }
}
