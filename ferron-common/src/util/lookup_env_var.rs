use std::env;

pub fn lookup_env_value(conf_string: String) -> String {
    let index_lb = conf_string.find("{");
    if let Some(index_lb) = index_lb {
        let index_rb_afterlb = conf_string.find("}");
        if let Some(index_rb_afterlb) = index_rb_afterlb {
            // +1 for {
            // +4 for env:
            return get_env_val(conf_string[index_lb + 5..index_rb_afterlb].to_string());
        } else {
            return conf_string;
        }
    } else {
        return conf_string;
    }
}

pub fn get_env_val(conf_key: String) -> String {
    match env::var(conf_key.clone()) {
        Ok(val) => return val,
        Err(e) => format!("couldn't find ENV-Key >>{conf_key}<< {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lala_get_env_var() {
        let result = get_env_val("LALA".to_string());
        assert_eq!(result, "couldn't find ENV-Key >>LALA<< environment variable not found");
    }

    #[test]
    fn home_get_env_var() {
        let result = get_env_val("HOME".to_string());
        assert_ne!(result, "");
    }

    #[test]
    fn missing_get_env_key() {
        let result = lookup_env_value("LALA".to_string());
        assert_eq!(result, "LALA");
    }

    #[test]
    fn success_lookup_env_value() {
        let result = lookup_env_value("{env:HOME}".to_string());
        assert_ne!(result, "");
    }
    #[test]
    fn failed_lookup_env_value() {
        let result = lookup_env_value("{envA:HOME}".to_string());
        assert_eq!(result, "couldn't find ENV-Key >>:HOME<< environment variable not found");
    }
}
