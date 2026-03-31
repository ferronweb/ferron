pub mod adapter;
mod builder;
pub mod layer;
pub mod macros;
pub mod validator;

pub use builder::*;

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    sync::Arc,
};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationSpan {
    pub line: usize,
    pub column: usize,
    pub file: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfiguration {
    pub global_config: Arc<ServerConfigurationBlock>,
    pub ports: BTreeMap<String, Vec<ServerConfigurationPort>>, // the key would be the protocol name
}

/*
Host configuration
 |
 +-- Port/IP (TCP)
 +-- Port/IP (HTTP)
     |
     +- Host/Location/Conditional
        |
        +- Error
 */

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationPort {
    pub port: u16,
    pub hosts: Vec<(ServerConfigurationHostFilters, ServerConfigurationBlock)>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationHostFilters {
    pub ip: Option<IpAddr>,
    pub host: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationBlock {
    pub directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    pub matchers: HashMap<String, ServerConfigurationMatcher>,
    pub span: Option<ServerConfigurationSpan>,
}

impl ServerConfigurationBlock {
    pub fn get_value(&self, directive: &str) -> Option<&ServerConfigurationValue> {
        self.directives
            .get(directive)
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.args.first())
    }

    pub fn get_flag(&self, directive: &str) -> bool {
        if let Some(v) = self.get_value(directive) {
            v.as_boolean().unwrap_or(true)
        } else {
            false
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationDirectiveEntry {
    pub args: Vec<ServerConfigurationValue>,
    pub children: Option<ServerConfigurationBlock>,
    pub span: Option<ServerConfigurationSpan>,
}

impl ServerConfigurationDirectiveEntry {
    pub fn get_value(&self) -> Option<&ServerConfigurationValue> {
        self.args.first()
    }

    pub fn get_flag(&self) -> bool {
        if let Some(ServerConfigurationValue::Boolean(value, _)) = self.args.first() {
            *value
        } else {
            true
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationValue {
    String(String, Option<ServerConfigurationSpan>),
    Number(i64, Option<ServerConfigurationSpan>),
    Float(f64, Option<ServerConfigurationSpan>),
    Boolean(bool, Option<ServerConfigurationSpan>),
    InterpolatedString(
        Vec<ServerConfigurationInterpolatedStringPart>,
        Option<ServerConfigurationSpan>,
    ),
}

impl ServerConfigurationValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ServerConfigurationValue::String(s, _) => Some(s),
            _ => None,
        }
    }

    pub fn as_string_with_interpolations(
        &self,
        variables: &HashMap<String, String>,
    ) -> Option<String> {
        match self {
            ServerConfigurationValue::String(s, _) => Some(s.clone()),
            ServerConfigurationValue::InterpolatedString(parts, _) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        ServerConfigurationInterpolatedStringPart::String(s) => result.push_str(s),
                        ServerConfigurationInterpolatedStringPart::Variable(var) => {
                            if let Some(value) = variables.get(var) {
                                result.push_str(value);
                            } else {
                                result.push_str(&format!("{{{{{}}}}}", var));
                            }
                        }
                    }
                }
                Some(result)
            }
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<i64> {
        if let ServerConfigurationValue::Number(n, _) = self {
            Some(*n)
        } else {
            None
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        if let ServerConfigurationValue::Float(f, _) = self {
            Some(*f)
        } else {
            None
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        if let ServerConfigurationValue::Boolean(b, _) = self {
            Some(*b)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationInterpolatedStringPart {
    String(String),
    Variable(String),
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationMatcher {
    pub exprs: Vec<ServerConfigurationMatcherExpr>,
    pub span: Option<ServerConfigurationSpan>,
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationMatcherExpr {
    pub left: ServerConfigurationMatcherOperand,
    pub right: ServerConfigurationMatcherOperand,
    pub op: ServerConfigurationMatcherOperator,
}

#[derive(Debug, PartialEq, PartialOrd, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationMatcherOperand {
    Identifier(String),
    String(String),
    Integer(i64),
    Float(f64),
}

impl Eq for ServerConfigurationMatcherOperand {}

impl Ord for ServerConfigurationMatcherOperand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use ServerConfigurationMatcherOperand::*;
        match (self, other) {
            (Identifier(a), Identifier(b)) => a.cmp(b),
            (String(a), String(b)) => a.cmp(b),
            (Integer(a), Integer(b)) => a.cmp(b),
            // For floats, we need to handle NaN values which do not have a total ordering
            (Float(a), Float(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),

            // Define an arbitrary but consistent ordering between different types
            (Identifier(_), _) => std::cmp::Ordering::Less,
            (String(_), Identifier(_)) => std::cmp::Ordering::Greater,
            (String(_), _) => std::cmp::Ordering::Less,
            (Integer(_), Identifier(_) | String(_)) => std::cmp::Ordering::Greater,
            (Integer(_), Float(_)) => std::cmp::Ordering::Less,
            (Float(_), Identifier(_) | String(_) | Integer(_)) => std::cmp::Ordering::Greater,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationMatcherOperator {
    Eq,
    NotEq,
    Regex,
    NotRegex,
    In,
}
