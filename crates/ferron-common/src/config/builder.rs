use super::{
    ServerConfiguration, ServerConfigurationBlock, ServerConfigurationDirectiveEntry,
    ServerConfigurationHostFilters, ServerConfigurationMatcher, ServerConfigurationMatcherExpr,
    ServerConfigurationMatcherOperand, ServerConfigurationMatcherOperator, ServerConfigurationPort,
    ServerConfigurationSpan, ServerConfigurationValue,
};
use std::{collections::BTreeMap, net::IpAddr, sync::Arc};

/// Builder for constructing [`ServerConfiguration`] instances.
pub struct ServerConfigurationBuilder {
    global_config: Option<Arc<ServerConfigurationBlock>>,
    ports: BTreeMap<String, ServerConfigurationPortBuilder>,
}

impl ServerConfigurationBuilder {
    /// Creates a new [`ServerConfigurationBuilder`] with default values.
    pub fn new() -> Self {
        Self {
            global_config: None,
            ports: BTreeMap::new(),
        }
    }

    /// Sets the global configuration block.
    pub fn global_config(mut self, config: ServerConfigurationBlock) -> Self {
        self.global_config = Some(Arc::new(config));
        self
    }

    /// Sets the global configuration block from an [`Arc`].
    pub fn global_config_arc(mut self, config: Arc<ServerConfigurationBlock>) -> Self {
        self.global_config = Some(config);
        self
    }

    /// Adds a port configuration to the builder.
    pub fn port(mut self, protocol: impl Into<String>, port: ServerConfigurationPort) -> Self {
        self.ports.insert(
            protocol.into(),
            ServerConfigurationPortBuilder { inner: port },
        );
        self
    }

    /// Adds a port configuration using a builder.
    pub fn port_with_builder(
        mut self,
        protocol: impl Into<String>,
        builder: ServerConfigurationPortBuilder,
    ) -> Self {
        self.ports.insert(protocol.into(), builder);
        self
    }

    /// Builds the [`ServerConfiguration`].
    ///
    /// # Panics
    /// Panics if the global configuration has not been set.
    pub fn build(self) -> ServerConfiguration {
        let global_config = self
            .global_config
            .expect("global_config must be set before building ServerConfiguration");

        let ports = self
            .ports
            .into_iter()
            .map(|(protocol, builder)| (protocol, builder.build()))
            .collect();

        ServerConfiguration {
            global_config,
            ports,
        }
    }

    /// Builds the [`ServerConfiguration`], returning an error if required fields are missing.
    pub fn try_build(self) -> Result<ServerConfiguration, ServerConfigurationBuilderError> {
        let global_config = self
            .global_config
            .ok_or(ServerConfigurationBuilderError::MissingGlobalConfig)?;

        let ports = self
            .ports
            .into_iter()
            .map(|(protocol, builder)| (protocol, builder.build()))
            .collect();

        Ok(ServerConfiguration {
            global_config,
            ports,
        })
    }
}

impl Default for ServerConfigurationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Error type for [`ServerConfigurationBuilder::try_build`].
#[derive(Debug, Clone)]
pub enum ServerConfigurationBuilderError {
    MissingGlobalConfig,
}

impl std::fmt::Display for ServerConfigurationBuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerConfigurationBuilderError::MissingGlobalConfig => {
                write!(
                    f,
                    "global_config must be set before building ServerConfiguration"
                )
            }
        }
    }
}

impl std::error::Error for ServerConfigurationBuilderError {}

/// Builder for constructing [`ServerConfigurationPort`] instances.
pub struct ServerConfigurationPortBuilder {
    inner: ServerConfigurationPort,
}

impl ServerConfigurationPortBuilder {
    /// Creates a new [`ServerConfigurationPortBuilder`] with the specified port number.
    pub fn new(port: u16) -> Self {
        Self {
            inner: ServerConfigurationPort {
                port,
                hosts: Vec::new(),
            },
        }
    }

    /// Adds a host configuration to this port.
    pub fn host(
        mut self,
        filters: ServerConfigurationHostFilters,
        block: ServerConfigurationBlock,
    ) -> Self {
        self.inner.hosts.push((filters, block));
        self
    }

    /// Adds a host configuration using a builder for the block.
    pub fn host_with_builder(
        mut self,
        filters: ServerConfigurationHostFilters,
        block_builder: ServerConfigurationBlockBuilder,
    ) -> Self {
        self.inner.hosts.push((filters, block_builder.build()));
        self
    }

    /// Builds the [`ServerConfigurationPort`].
    pub fn build(self) -> ServerConfigurationPort {
        self.inner
    }
}

/// Builder for constructing [`ServerConfigurationBlock`] instances.
pub struct ServerConfigurationBlockBuilder {
    directives: Vec<(String, ServerConfigurationDirectiveEntry)>,
    matchers: Vec<(String, ServerConfigurationMatcher)>,
    span: Option<ServerConfigurationSpan>,
}

impl ServerConfigurationBlockBuilder {
    /// Creates a new [`ServerConfigurationBlockBuilder`] with default values.
    pub fn new() -> Self {
        Self {
            directives: Vec::new(),
            matchers: Vec::new(),
            span: None,
        }
    }

    /// Sets the span information for this block.
    pub fn span(mut self, line: usize, column: usize) -> Self {
        self.span = Some(ServerConfigurationSpan { line, column });
        self
    }

    /// Sets the span from an existing [`ServerConfigurationSpan`].
    pub fn span_opt(mut self, span: Option<ServerConfigurationSpan>) -> Self {
        self.span = span;
        self
    }

    /// Adds a directive to this block.
    pub fn directive(
        mut self,
        name: impl Into<String>,
        entry: ServerConfigurationDirectiveEntry,
    ) -> Self {
        self.directives.push((name.into(), entry));
        self
    }

    /// Adds a directive with simple string arguments.
    pub fn directive_str(self, name: impl Into<String>, args: Vec<impl Into<String>>) -> Self {
        let entry = ServerConfigurationDirectiveEntry {
            args: args
                .into_iter()
                .map(|s| ServerConfigurationValue::String(s.into(), None))
                .collect(),
            children: None,
            span: None,
        };
        self.directive(name, entry)
    }

    /// Adds a directive with a nested block.
    pub fn directive_with_block(
        self,
        name: impl Into<String>,
        args: Vec<impl Into<String>>,
        block: ServerConfigurationBlock,
    ) -> Self {
        let entry = ServerConfigurationDirectiveEntry {
            args: args
                .into_iter()
                .map(|s| ServerConfigurationValue::String(s.into(), None))
                .collect(),
            children: Some(block),
            span: None,
        };
        self.directive(name, entry)
    }

    /// Adds a directive with a nested block using a builder.
    pub fn directive_with_block_builder(
        self,
        name: impl Into<String>,
        args: Vec<impl Into<String>>,
        block_builder: ServerConfigurationBlockBuilder,
    ) -> Self {
        let entry = ServerConfigurationDirectiveEntry {
            args: args
                .into_iter()
                .map(|s| ServerConfigurationValue::String(s.into(), None))
                .collect(),
            children: Some(block_builder.build()),
            span: None,
        };
        self.directive(name, entry)
    }

    /// Adds a matcher to this block.
    pub fn matcher(mut self, name: impl Into<String>, matcher: ServerConfigurationMatcher) -> Self {
        self.matchers.push((name.into(), matcher));
        self
    }

    /// Adds a simple equality matcher.
    pub fn matcher_eq(
        self,
        name: impl Into<String>,
        identifier: impl Into<String>,
        value: impl Into<String>,
        span: ServerConfigurationSpan,
    ) -> Self {
        let matcher = ServerConfigurationMatcher {
            exprs: vec![ServerConfigurationMatcherExpr {
                left: ServerConfigurationMatcherOperand::Identifier(
                    identifier.into(),
                    Some(span.clone()),
                ),
                right: ServerConfigurationMatcherOperand::String(value.into(), Some(span)),
                op: ServerConfigurationMatcherOperator::Eq,
            }],
            span: None,
        };
        self.matcher(name, matcher)
    }

    /// Builds the [`ServerConfigurationBlock`].
    pub fn build(self) -> ServerConfigurationBlock {
        let mut directives_map = std::collections::HashMap::new();
        for (name, entry) in self.directives {
            directives_map
                .entry(name)
                .or_insert_with(Vec::new)
                .push(entry);
        }

        let mut matchers_map = std::collections::HashMap::new();
        for (name, matcher) in self.matchers {
            matchers_map
                .entry(name)
                .or_insert_with(Vec::new)
                .push(matcher);
        }

        ServerConfigurationBlock {
            directives: directives_map,
            matchers: matchers_map,
            span: self.span,
        }
    }
}

impl Default for ServerConfigurationBlockBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing [`ServerConfigurationHostFilters`] instances.
pub struct ServerConfigurationHostFiltersBuilder {
    ip: Option<IpAddr>,
    host: Option<String>,
}

impl ServerConfigurationHostFiltersBuilder {
    /// Creates a new [`ServerConfigurationHostFiltersBuilder`] with default values.
    pub fn new() -> Self {
        Self {
            ip: None,
            host: None,
        }
    }

    /// Sets the IP address filter.
    pub fn ip(mut self, ip: IpAddr) -> Self {
        self.ip = Some(ip);
        self
    }

    /// Sets the host name filter.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Builds the [`ServerConfigurationHostFilters`].
    pub fn build(self) -> ServerConfigurationHostFilters {
        ServerConfigurationHostFilters {
            ip: self.ip,
            host: self.host,
        }
    }
}

impl Default for ServerConfigurationHostFiltersBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing [`ServerConfigurationMatcher`] instances.
pub struct ServerConfigurationMatcherBuilder {
    exprs: Vec<ServerConfigurationMatcherExpr>,
    span: Option<ServerConfigurationSpan>,
}

impl ServerConfigurationMatcherBuilder {
    /// Creates a new [`ServerConfigurationMatcherBuilder`] with default values.
    pub fn new() -> Self {
        Self {
            exprs: Vec::new(),
            span: None,
        }
    }

    /// Sets the span information for this matcher.
    pub fn span(mut self, line: usize, column: usize) -> Self {
        self.span = Some(ServerConfigurationSpan { line, column });
        self
    }

    /// Adds an expression to this matcher.
    pub fn expr(mut self, expr: ServerConfigurationMatcherExpr) -> Self {
        self.exprs.push(expr);
        self
    }

    /// Adds an equality expression.
    pub fn expr_eq(
        mut self,
        identifier: impl Into<String>,
        value: impl Into<String>,
        span_l: Option<ServerConfigurationSpan>,
        span_r: Option<ServerConfigurationSpan>,
    ) -> Self {
        self.exprs.push(ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier(identifier.into(), span_l),
            right: ServerConfigurationMatcherOperand::String(value.into(), span_r),
            op: ServerConfigurationMatcherOperator::Eq,
        });
        self
    }

    /// Adds a not-equal expression.
    pub fn expr_not_eq(
        mut self,
        identifier: impl Into<String>,
        value: impl Into<String>,
        span_l: Option<ServerConfigurationSpan>,
        span_r: Option<ServerConfigurationSpan>,
    ) -> Self {
        self.exprs.push(ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier(identifier.into(), span_l),
            right: ServerConfigurationMatcherOperand::String(value.into(), span_r),
            op: ServerConfigurationMatcherOperator::NotEq,
        });
        self
    }

    /// Adds a regex expression.
    pub fn expr_regex(
        mut self,
        identifier: impl Into<String>,
        pattern: impl Into<String>,
        span_l: Option<ServerConfigurationSpan>,
        span_r: Option<ServerConfigurationSpan>,
    ) -> Self {
        self.exprs.push(ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier(identifier.into(), span_l),
            right: ServerConfigurationMatcherOperand::String(pattern.into(), span_r),
            op: ServerConfigurationMatcherOperator::Regex,
        });
        self
    }

    /// Builds the [`ServerConfigurationMatcher`].
    pub fn build(self) -> ServerConfigurationMatcher {
        ServerConfigurationMatcher {
            exprs: self.exprs,
            span: self.span,
        }
    }
}

impl Default for ServerConfigurationMatcherBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper for creating [`ServerConfigurationValue`] instances.
pub struct ServerConfigurationValueBuilder;

impl ServerConfigurationValueBuilder {
    /// Creates a string value.
    pub fn string(s: impl Into<String>) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.into(), None)
    }

    /// Creates a string value with span information.
    pub fn string_with_span(
        s: impl Into<String>,
        line: usize,
        column: usize,
    ) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.into(), Some(ServerConfigurationSpan { line, column }))
    }

    /// Creates a number value.
    pub fn number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

    /// Creates a number value with span information.
    pub fn number_with_span(n: i64, line: usize, column: usize) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, Some(ServerConfigurationSpan { line, column }))
    }

    /// Creates a float value.
    pub fn float(f: f64) -> ServerConfigurationValue {
        ServerConfigurationValue::Float(f, None)
    }

    /// Creates a float value with span information.
    pub fn float_with_span(f: f64, line: usize, column: usize) -> ServerConfigurationValue {
        ServerConfigurationValue::Float(f, Some(ServerConfigurationSpan { line, column }))
    }

    /// Creates a boolean value.
    pub fn boolean(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    /// Creates a boolean value with span information.
    pub fn boolean_with_span(b: bool, line: usize, column: usize) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, Some(ServerConfigurationSpan { line, column }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_configuration_builder() {
        let config = ServerConfigurationBuilder::new()
            .global_config(
                ServerConfigurationBlockBuilder::new()
                    .directive_str("listen", vec!["8080"])
                    .build(),
            )
            .port_with_builder(
                "http",
                ServerConfigurationPortBuilder::new(8080).host(
                    ServerConfigurationHostFiltersBuilder::new()
                        .host("example.com")
                        .build(),
                    ServerConfigurationBlockBuilder::new()
                        .directive_str("root", vec!["/var/www/html"])
                        .build(),
                ),
            )
            .build();

        assert_eq!(config.ports.len(), 1);
        assert!(config.ports.contains_key("http"));
    }

    #[test]
    fn test_matcher_builder() {
        let span = ServerConfigurationSpan { line: 1, column: 0 };

        let matcher = ServerConfigurationMatcherBuilder::new()
            .expr_eq(
                "host",
                "example.com",
                Some(span.clone()),
                Some(span.clone()),
            )
            .expr_not_eq("path", "/admin", Some(span.clone()), Some(span.clone()))
            .build();

        assert_eq!(matcher.exprs.len(), 2);
    }

    #[test]
    fn test_value_builder() {
        let string_val = ServerConfigurationValueBuilder::string("hello");
        let number_val = ServerConfigurationValueBuilder::number(42);
        let bool_val = ServerConfigurationValueBuilder::boolean(true);

        assert!(matches!(string_val, ServerConfigurationValue::String(_, _)));
        assert!(matches!(number_val, ServerConfigurationValue::Number(_, _)));
        assert!(matches!(bool_val, ServerConfigurationValue::Boolean(_, _)));
    }
}
