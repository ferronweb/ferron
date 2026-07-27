use crate::{
    directives::{Directive, DirectiveSubblock},
    loader::ModuleLoader,
};

mod validator;

pub use validator::*;

#[derive(Default)]
pub struct BuiltinModuleLoader;

impl ModuleLoader for BuiltinModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn crate::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(validator::BuiltinConfigurationValidator));
    }

    fn register_directives(&mut self, registry: &mut crate::directives::DirectiveRegistry) {
        registry
            .register(
                Directive {
                    name: "runtime",
                    usage: "runtime { ... }",
                    description: "This directive specifies global runtime settings.",
                    applicable_protocols: None,
                    global_only: true,
                    subblock_link: Some(DirectiveSubblock::custom("global_runtime")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "io_uring",
                    usage: "io_uring [bool]",
                    description: "This directive specifies whether `io_uring` is enabled for the \
                primary runtime when available. If initialization fails, Ferron falls back to \
                epoll and logs a warning. Default: enabled",
                    applicable_protocols: None,
                    global_only: true,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("global_runtime"),
            )
            .register(
                Directive {
                    name: "tcp",
                    usage: "tcp { ... }",
                    description: "This directive specifies global TCP settings for HTTP \
                listeners.",
                    applicable_protocols: None,
                    global_only: true,
                    subblock_link: Some(DirectiveSubblock::custom("tcp")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "listen",
                    usage: "listen <address>",
                    description: "This directive specifies the listener bind address for HTTP \
                TCP listeners. Accepts either an IP address or a full socket address. \
                Default: [::]:<http-port>",
                    applicable_protocols: None,
                    global_only: true,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("tcp"),
            )
            .register(
                Directive {
                    name: "send_buf",
                    usage: "send_buf <size>",
                    description: "This directive specifies the TCP send buffer size. Must \
                resolve to a non-negative integer at runtime. Default: OS default",
                    applicable_protocols: None,
                    global_only: true,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("tcp"),
            )
            .register(
                Directive {
                    name: "recv_buf",
                    usage: "recv_buf <size>",
                    description: "This directive specifies the TCP receive buffer size. Must \
                resolve to a non-negative integer at runtime. Default: OS default",
                    applicable_protocols: None,
                    global_only: true,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("tcp"),
            )
            .register(
                Directive {
                    name: "backlog",
                    usage: "backlog <size>",
                    description: "This directive specifies the maximum number of pending \
                connections allowed on the listener socket. Default: -1 (unlimited)",
                    applicable_protocols: None,
                    global_only: true,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("tcp"),
            )
            .register(
                Directive {
                    name: "multipath",
                    usage: "multipath [bool]",
                    description: "This directive specifies whether Multipath TCP (MPTCP) is \
                enabled for the listener. MPTCP allows a single TCP connection to use multiple \
                network interfaces simultaneously, improving throughput and resilience. When \
                enabled, Ferron attempts to create an MPTCP socket; if the kernel does not \
                support MPTCP or it is disabled, a warning is logged and the listener falls \
                back to standard TCP. Default: disabled",
                    applicable_protocols: None,
                    global_only: true,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("tcp"),
            )
            .register(
                Directive {
                    name: "observability",
                    usage: "observability [bool] | observability { ... }",
                    description: "This directive configures per-host event sinks for logging \
                and metrics. Multiple observability directives for the same host accumulate \
                event sinks.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("observability")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "log",
                    usage: "log <path> [bool] | log <path> { ... }",
                    description: "This directive is shorthand for configuring access logging \
                with the file provider. Automatically transformed into an equivalent \
                observability block.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("observability")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "error_log",
                    usage: "error_log <path> [bool] | error_log <path> { ... }",
                    description: "This directive is shorthand for configuring error logging \
                with the file provider. Automatically transformed into an equivalent \
                observability block.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("observability")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "console_log",
                    usage: "console_log [bool] | console_log { ... }",
                    description: "This directive is shorthand for configuring console-based \
                observability. Automatically transformed into an equivalent \
                observability block.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("observability")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "control_plane",
                    usage: "control_plane { ... }",
                    description: "This directive embeds contextual metadata and static \
                OpenTelemetry span links from the server configuration into all \
                observability signals (traces, logs, metrics, access logs).",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("control_plane")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "metadata",
                    usage: "metadata { <key> <value> ... }",
                    description: "This sub-block holds arbitrary key-value pairs injected as \
                `ferron.control_plane.*` attributes on all observability signals.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("control_plane"),
            )
            .register(
                Directive {
                    name: "span_links",
                    usage: "span_links { trace_id ...; span_id ...; ... }",
                    description: "This sub-block defines static OpenTelemetry span links \
                attached to every `ferron.request` span, creating causal connections to \
                control plane traces.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("control_plane"),
            )
            .register(
                Directive {
                    name: "trace_id",
                    usage: "trace_id <string>",
                    description: "This directive specifies the 32-hex-character trace ID of \
                the linked span.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("control_plane_span_links"),
            )
            .register(
                Directive {
                    name: "span_id",
                    usage: "span_id <string>",
                    description: "This directive specifies the 16-hex-character span ID of \
                the linked span.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("control_plane_span_links"),
            )
            .register(
                Directive {
                    name: "sampled",
                    usage: "sampled [bool]",
                    description: "This directive specifies whether the linked span was \
                sampled. Default: false",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("control_plane_span_links"),
            )
            .register(
                Directive {
                    name: "attributes",
                    usage: "attributes { <key> <value> ... }",
                    description: "This sub-block holds key-value pairs describing the \
                relationship of the span link.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("control_plane_span_links"),
            );
    }
}
