use std::env;

fn resolve_placeholder(kind: &str, value: &str) -> Option<String> {
    match kind {
        "env" => match env::var(value) {
            Ok(val) => Some(val),
            Err(_e) => None,
        },
        _ => None,
    }
}

pub fn replace_placeholders(input: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    
    while cursor < input.len() {
        // Find the next opening brace
        let index_lb = input[cursor..].find('{');
        
        let index_lb = match index_lb {
            Some(pos) => pos,
            None => {
                // No more placeholders, push remaining text and break
                output.push_str(&input[cursor..]);
                break;
            }
        };
        
        // Look for closing brace after the opening brace
        let after_lb = cursor + index_lb + 1;
        if after_lb <= input.len() {
            let index_rb_afterlb = input[after_lb..].find('}');
            
            match index_rb_afterlb {
                Some(index_rb_offset) => {
                    let index_rb = after_lb + index_rb_offset;
                    let placeholder = &input[after_lb..index_rb];
                    
                    // Push text before this placeholder
                    output.push_str(&input[cursor..cursor + index_lb]);
                    
                    // Try to resolve the placeholder
                    if let Some((kind, value)) = placeholder.split_once(':') {
                        match resolve_placeholder(kind, value) {
                            Some(resolved) => output.push_str(&resolved),
                            None => {
                                // Keep original placeholder
                                output.push('{');
                                output.push_str(placeholder);
                                output.push('}');
                            }
                        }
                    } else {
                        // No colon, keep original
                        output.push('{');
                        output.push_str(placeholder);
                        output.push('}');
                    }
                    
                    cursor = index_rb + 1;
                }
                None => {
                    // No closing brace found, push the rest of the string as-is
                    output.push_str(&input[cursor..]);
                    break;
                }
            }
        } else {
            // Shouldn't happen as after_lb is within bounds
            output.push_str(&input[cursor..]);
            break;
        }
    }
    
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn env_var_missing() {
        let result = resolve_placeholder("env", "LALA_SHOULD_NOT_EXIST");
        assert_eq!(result, None);
    }

    #[test]
    fn env_var_exists() {
        env::set_var("TEST_ENV_EXISTS", "value");

        let result = resolve_placeholder("env", "TEST_ENV_EXISTS");
        assert_eq!(result, Some("value".to_string()));
    }

    #[test]
    fn passthrough_no_placeholders() {
        let input = "LALA";
        let result = replace_placeholders(input);
        assert_eq!(result, input);
    }

    #[test]
    fn single_env_placeholder() {
        env::set_var("TEST_HOME", "/home/test");

        let result = replace_placeholders("{env:TEST_HOME}");
        assert_eq!(result, "/home/test");
    }

    #[test]
    fn unknown_kind_passthrough() {
        let result = replace_placeholders("{envA:HOME}");
        assert_eq!(result, "{envA:HOME}");
    }

    #[test]
    fn interpolate_env_with_suffix() {
        env::set_var("TEST_HOME", "/home/test");

        let result = replace_placeholders("{env:TEST_HOME}/src/modules");
        assert_eq!(result, "/home/test/src/modules");
    }

    #[test]
    fn interpolate_multiple_env_values() {
        env::set_var("TEST_HOME", "/home/test");
        env::set_var("TEST_USER", "user");

        let input = "prefix_{env:TEST_HOME}_middle_{env:TEST_USER}_suffix";
        let result = replace_placeholders(input);

        let expected = "prefix_/home/test_middle_user_suffix";
        assert_eq!(result, expected);
    }

    #[test]
    fn plain_string_passthrough() {
        let input = "plain_string_without_env";
        let result = replace_placeholders(input);
        assert_eq!(result, input);
    }

    #[test]
    fn missing_closing_brace() {
        let result = replace_placeholders("{env:TEST_HOME");
        // Reference behavior: push all remaining text including the unmatched '{'
        assert_eq!(result, "{env:TEST_HOME");
    }

    #[test]
    fn missing_closing_brace_with_text_after() {
        let result = replace_placeholders("{env:TEST_HOME and then more text");
        // Should keep everything including the opening brace
        assert_eq!(result, "{env:TEST_HOME and then more text");
    }

    #[test]
    fn nested_or_multiple_braces() {
        let result = replace_placeholders("text {first} middle {second");
        // Second has no closing brace, so everything from first { to end stays
        assert_eq!(result, "text {first} middle {second");
    }

    #[test]
    fn nonexistent_env_var() {
        let result = replace_placeholders("{env:THIS_SHOULD_NOT_EXIST_123}");
        assert_eq!(result, "{env:THIS_SHOULD_NOT_EXIST_123}");
    }

    #[test]
    fn nonexistent_env_var_in_interpolation() {
        let result = replace_placeholders("prefix_{env:THIS_SHOULD_NOT_EXIST_456}_suffix");
        assert_eq!(result, "prefix_{env:THIS_SHOULD_NOT_EXIST_456}_suffix");
    }

    #[test]
    fn placeholder_without_colon_passthrough() {
        let result = replace_placeholders("{justtext}");
        assert_eq!(result, "{justtext}");
    }

    // Note: The reference implementation doesn't support escaping,
    // so backslashes are treated as literal characters
    
    #[test]
    fn backslash_before_brace_is_literal() {
        // Reference implementation doesn't escape, so \{ is just two chars
        let result = replace_placeholders(r"\{env:TEST_HOME}");
        assert_eq!(result, r"\{env:TEST_HOME}");
    }

    #[test]
    fn backslash_not_special() {
        let result = replace_placeholders(r"prefix_\{env:TEST_HOME}_suffix");
        assert_eq!(result, r"prefix_\{env:TEST_HOME}_suffix");
    }
    
    #[test]
    fn multiple_placeholders_mixed_with_missing() {
        env::set_var("EXISTS", "found");
        let result = replace_placeholders("{env:EXISTS} then {missing");
        assert_eq!(result, "found then {missing");
    }
}
