pub mod adapter;
mod builder;
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
    pub matchers: HashMap<String, Vec<ServerConfigurationMatcher>>,
    pub span: Option<ServerConfigurationSpan>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationDirectiveEntry {
    pub args: Vec<ServerConfigurationValue>,
    pub children: Option<ServerConfigurationBlock>,
    pub span: Option<ServerConfigurationSpan>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationMatcherExpr {
    pub left: ServerConfigurationMatcherOperand,
    pub right: ServerConfigurationMatcherOperand,
    pub op: ServerConfigurationMatcherOperator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationMatcherOperand {
    Identifier(String, Option<ServerConfigurationSpan>),
    String(String, Option<ServerConfigurationSpan>),
    Integer(i64, Option<ServerConfigurationSpan>),
    Float(f64, Option<ServerConfigurationSpan>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationMatcherOperator {
    Eq,
    NotEq,
    Regex,
    NotRegex,
    In,
}
