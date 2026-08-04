pub(crate) fn register_core_proxy_directives(
    registry: &mut ferron_core::directives::DirectiveRegistry,
) {
    use ferron_core::directives::DirectiveSubblock;
    register_directive_with_link(registry, "proxy", "proxy <url> | proxy { ... }", "This directive enables reverse proxying with upstream, circuit breaker, retry, affinity, and connection settings.", subblock("http_proxy"), DirectiveSubblock::default());
    register_directive(
        registry,
        "proxy_concurrent_conns",
        "proxy_concurrent_conns <limit>",
        "This directive sets the global or per-host limit for concurrent proxy connections.",
        DirectiveSubblock::default(),
    );
    register_directive_with_link(registry, "upstream", "upstream <url> { ... }", "This directive defines an upstream backend with connection limits, TLS, DNS, and active health check settings.", subblock("http_proxy_upstream"), subblock("http_proxy"));
    register_directive(
        registry,
        "algorithm",
        "algorithm <name>",
        "This directive selects the load balancing algorithm for proxy upstreams.",
        subblock("http_proxy"),
    );
}

#[inline]
pub(crate) fn register_health_check_directives(
    registry: &mut ferron_core::directives::DirectiveRegistry,
) {
    register_directive_with_link(
        registry,
        "active_check",
        "active_check [bool] | active_check { ... }",
        "This directive configures active health checks for upstream backends.",
        subblock("http_proxy_active_check"),
        subblock("http_proxy_upstream"),
    );
    register_directive(
        registry,
        "uri",
        "uri <path>",
        "This directive sets the URI for active health check requests.",
        subblock("http_proxy_active_check"),
    );
    register_directive(
        registry,
        "expect_status",
        "expect_status <pattern>",
        "This directive sets the expected status pattern for active health checks.",
        subblock("http_proxy_active_check"),
    );
    register_directive(
        registry,
        "interval",
        "interval <duration>",
        "This directive sets the health check interval.",
        subblock("http_proxy_active_check"),
    );
    register_directive(
        registry,
        "timeout",
        "timeout <duration>",
        "This directive sets the health check timeout.",
        subblock("http_proxy_active_check"),
    );
    register_directive(
        registry,
        "method",
        "method <name>",
        "This directive sets the HTTP method for active health checks.",
        subblock("http_proxy_active_check"),
    );
    register_directive(
        registry,
        "response_time_threshold",
        "response_time_threshold <duration>",
        "This directive sets the response time threshold for health checks.",
        subblock("http_proxy_active_check"),
    );
    register_directive(
        registry,
        "body_match",
        "body_match <substring>",
        "This directive sets a substring to match in active health check responses.",
        subblock("http_proxy_active_check"),
    );
    register_directive(
        registry,
        "consecutive_fails",
        "consecutive_fails <count>",
        "This directive sets the consecutive failure count to mark upstream as down.",
        subblock("http_proxy_active_check"),
    );
    register_directive(
        registry,
        "consecutive_passes",
        "consecutive_passes <count>",
        "This directive sets the consecutive pass count to mark upstream as healthy.",
        subblock("http_proxy_active_check"),
    );
}

#[inline]
pub(crate) fn register_upstream_connection_directives(
    registry: &mut ferron_core::directives::DirectiveRegistry,
) {
    register_directive(
        registry,
        "cert",
        "cert <path>",
        "This directive sets the TLS client certificate path for upstream connections.",
        subblock("http_proxy_upstream"),
    );
    register_directive(
        registry,
        "key",
        "key <path>",
        "This directive sets the TLS client key path for upstream connections.",
        subblock("http_proxy_upstream"),
    );
    register_directive(
        registry,
        "idle_timeout",
        "idle_timeout <duration>",
        "This directive sets the idle timeout for upstream connections.",
        subblock("http_proxy_upstream"),
    );
    register_directive(
        registry,
        "connection_timeout",
        "connection_timeout <duration>",
        "This directive sets the connection timeout for upstream backends.",
        subblock("http_proxy_upstream"),
    );
    register_directive(
        registry,
        "unix",
        "unix <path>",
        "This directive sets a Unix socket path for upstream connections.",
        subblock("http_proxy_upstream"),
    );
    register_directive(
        registry,
        "weight",
        "weight <value>",
        "This directive sets the load balancing weight for an upstream backend.",
        subblock("http_proxy_upstream"),
    );
    register_directive(
        registry,
        "priority",
        "priority <value>",
        "This directive sets the priority for an upstream backend.",
        subblock("http_proxy_upstream"),
    );
    register_directive(
        registry,
        "logical_dns",
        "logical_dns [bool]",
        "This directive enables logical DNS resolution for upstream backends.",
        subblock("http_proxy_upstream"),
    );
    register_directive(
        registry,
        "dns_servers",
        "dns_servers <servers>",
        "This directive sets custom DNS servers for upstream backend resolution.",
        subblock("http_proxy_upstream"),
    );
}

#[inline]
pub(crate) fn register_circuit_breaker_directives(
    registry: &mut ferron_core::directives::DirectiveRegistry,
) {
    register_directive_with_link(registry, "circuit_breaker", "circuit_breaker [bool] | circuit_breaker { ... }", "This directive configures circuit breaker with fail thresholds, window, and slow start settings.", subblock("http_proxy_circuit_breaker"), subblock("http_proxy"));
    register_directive(
        registry,
        "max_fails",
        "max_fails <count>",
        "This directive sets the maximum number of failures before circuit breaker opens.",
        subblock("http_proxy_circuit_breaker"),
    );
    register_directive(
        registry,
        "open_duration",
        "open_duration <duration>",
        "This directive sets how long the circuit breaker stays open.",
        subblock("http_proxy_circuit_breaker"),
    );
    register_directive(
        registry,
        "record_5xx",
        "record_5xx [bool]",
        "This directive enables recording 5xx status codes as circuit breaker failures.",
        subblock("http_proxy_circuit_breaker"),
    );
    register_directive(
        registry,
        "latency_threshold",
        "latency_threshold <duration>",
        "This directive sets the latency threshold for circuit breaker slow request detection.",
        subblock("http_proxy_circuit_breaker"),
    );
    register_directive(
        registry,
        "flapping_transitions",
        "flapping_transitions <count>",
        "This directive sets the number of state transitions that indicate flapping.",
        subblock("http_proxy_circuit_breaker"),
    );
    register_directive(
        registry,
        "flapping_window",
        "flapping_window <duration>",
        "This directive sets the observation window for flapping detection.",
        subblock("http_proxy_circuit_breaker"),
    );
    register_directive(
        registry,
        "slow_start",
        "slow_start <duration>",
        "This directive sets the slow-start duration after a circuit breaker recovers.",
        subblock("http_proxy_circuit_breaker"),
    );
    register_directive(
        registry,
        "window",
        "window <duration>",
        "This directive sets the observation window for circuit breaker or retry budget.",
        subblock("http_proxy_circuit_breaker"),
    );
}

#[inline]
pub(crate) fn register_retry_budget_directives(
    registry: &mut ferron_core::directives::DirectiveRegistry,
) {
    register_directive_with_link(registry, "retry_budget", "retry_budget [bool] | retry_budget { ... }", "This directive configures retry budget with max retry rate, token bucket, and refill settings.", subblock("http_proxy_retry_budget"), subblock("http_proxy"));
    register_directive(
        registry,
        "max_retry_rate",
        "max_retry_rate <ratio>",
        "This directive sets the maximum retry rate as a fraction of total requests.",
        subblock("http_proxy_retry_budget"),
    );
    register_directive(
        registry,
        "max_tokens",
        "max_tokens <count>",
        "This directive sets the maximum token count for retry budget.",
        subblock("http_proxy_retry_budget"),
    );
    register_directive(
        registry,
        "refill_rate",
        "refill_rate <rate>",
        "This directive sets the token refill rate for retry budget.",
        subblock("http_proxy_retry_budget"),
    );
    register_directive(
        registry,
        "retry_connection",
        "retry_connection [bool]",
        "This directive enables retrying on connection errors.",
        subblock("http_proxy"),
    );
}

#[inline]
pub(crate) fn register_connection_feature_directives(
    registry: &mut ferron_core::directives::DirectiveRegistry,
) {
    register_directive(
        registry,
        "keepalive",
        "keepalive [bool]",
        "This directive enables keepalive connections to upstream backends.",
        subblock("http_proxy"),
    );
    register_directive(
        registry,
        "http2",
        "http2 [bool]",
        "This directive enables HTTP/2 for upstream connections.",
        subblock("http_proxy"),
    );
    register_directive(
        registry,
        "http2_only",
        "http2_only [bool]",
        "This directive restricts upstream connections to HTTP/2 only.",
        subblock("http_proxy"),
    );
    register_directive(
        registry,
        "intercept_errors",
        "intercept_errors [bool]",
        "This directive enables interception of upstream error responses for custom handling.",
        subblock("http_proxy"),
    );
    register_directive(
        registry,
        "no_verification",
        "no_verification [bool]",
        "This directive disables TLS certificate verification for upstream connections.",
        subblock("http_proxy"),
    );
    register_directive(
        registry,
        "metrics_resolved_ip",
        "metrics_resolved_ip [bool]",
        "This directive enables reporting resolved upstream IPs in proxy metrics.",
        subblock("http_proxy"),
    );
    register_directive(
        registry,
        "proxy_header",
        "proxy_header <format>",
        "This directive sets the PROXY protocol version (v1 or v2) for upstream connections.",
        subblock("http_proxy"),
    );
}

#[inline]
pub(crate) fn register_affinity_directives(
    registry: &mut ferron_core::directives::DirectiveRegistry,
) {
    register_directive_with_link(
        registry,
        "affinity",
        "affinity <type> | affinity <type> { ... }",
        "This directive configures session affinity using cookie, header, IP, or hash methods.",
        subblock("http_proxy_affinity"),
        subblock("http_proxy"),
    );
    register_directive(registry, "name", "name <value>", "This directive sets the cookie name for cookie affinity or header name for header affinity.", subblock("http_proxy_affinity"));
    register_directive(
        registry,
        "ttl",
        "ttl <duration>",
        "This directive sets the TTL for cookie-based session affinity.",
        subblock("http_proxy_affinity"),
    );
    register_directive(
        registry,
        "path",
        "path <value>",
        "This directive sets the cookie path for cookie-based session affinity.",
        subblock("http_proxy_affinity"),
    );
    register_directive(
        registry,
        "domain",
        "domain <value>",
        "This directive sets the cookie domain for cookie-based session affinity.",
        subblock("http_proxy_affinity"),
    );
    register_directive(
        registry,
        "secure",
        "secure [bool]",
        "This directive sets the Secure flag on the affinity cookie.",
        subblock("http_proxy_affinity"),
    );
    register_directive(
        registry,
        "httponly",
        "httponly [bool]",
        "This directive sets the HttpOnly flag on the affinity cookie.",
        subblock("http_proxy_affinity"),
    );
    register_directive(
        registry,
        "samesite",
        "samesite <policy>",
        "This directive sets the SameSite policy for the affinity cookie.",
        subblock("http_proxy_affinity"),
    );
    register_directive(
        registry,
        "variable",
        "variable <name>",
        "This directive sets the variable name for hash-based session affinity.",
        subblock("http_proxy_affinity"),
    );
}

#[inline]
fn register_directive(
    registry: &mut ferron_core::directives::DirectiveRegistry,
    name: &'static str,
    usage: &'static str,
    description: &'static str,
    parent: ferron_core::directives::DirectiveSubblock,
) {
    use ferron_core::directives::Directive;
    registry.register(
        Directive {
            name,
            usage,
            description,
            applicable_protocols: Some(&["http"]),
            global_only: false,
            subblock_link: None,
        },
        parent,
    );
}

#[inline]
fn register_directive_with_link(
    registry: &mut ferron_core::directives::DirectiveRegistry,
    name: &'static str,
    usage: &'static str,
    description: &'static str,
    link: ferron_core::directives::DirectiveSubblock,
    parent: ferron_core::directives::DirectiveSubblock,
) {
    use ferron_core::directives::Directive;
    registry.register(
        Directive {
            name,
            usage,
            description,
            applicable_protocols: Some(&["http"]),
            global_only: false,
            subblock_link: Some(link),
        },
        parent,
    );
}

#[inline]
fn subblock(name: &'static str) -> ferron_core::directives::DirectiveSubblock {
    ferron_core::directives::DirectiveSubblock::custom(name)
}
