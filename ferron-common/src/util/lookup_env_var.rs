use std::env;
use anyhow::bail;

pub fn lookup_env_value(conf_string: String) -> anyhow::Result<String> {
    if !conf_string.starts_with("{env:") {
        bail!("String '{}' does not start with '{{env:'", conf_string);
    }

    // Search Index of "{env:"
    if let Some(index_lb) = conf_string.find("{env:") {
        // Search for ending "}"
        if let Some(index_rb_afterlb) = conf_string[index_lb + 5..].find("}") {
            return get_env_val(
                conf_string[index_lb + 5..index_lb + 5 + index_rb_afterlb].to_string(),
            );
        } else {
            bail!("No closing '}}' found in string '{}'", conf_string);
        }
    }

    // If not an env prefix return the string
    Ok(conf_string)
}

pub fn get_env_val(conf_key: String) -> anyhow::Result<String> {
    match env::var(conf_key.clone()) {
        Ok(val) => return Ok(val),
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
        let result = lookup_env_value("LALA".to_string());
        assert!(result.is_err(), "Not found LALA");
    }

    #[test]
    fn success_lookup_env_value() {
        let result = lookup_env_value("{env:HOME}".to_string());
        assert!(result.is_ok(), "Get successfully 'HOME' ENV");
    }
    #[test]
    fn failed_lookup_env_value() {
        let result = lookup_env_value("{envA:HOME}".to_string());
        assert!(
            result.is_err(),
            "couldn't find ENV-Key >>:HOME<< environment variable not found"
        );
    }
}
