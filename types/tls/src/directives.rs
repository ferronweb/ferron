use ferron_core::directives::{Directive, DirectiveRegistry, DirectiveSubblock};

pub fn register_tls_common_directives(
    registry: &mut DirectiveRegistry,
    subblock: DirectiveSubblock,
    applicable_protocols: Option<&'static [&'static str]>,
) {
    registry
        .register(
            Directive {
                name: "cert",
                usage: "cert <path>",
                description: "This directive specifies the path to the TLS certificate file \
            (PEM).",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            subblock,
        )
        .register(
            Directive {
                name: "key",
                usage: "key <path>",
                description: "This directive specifies the path to the TLS private key file \
            (PEM).",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            subblock,
        )
        .register(
            Directive {
                name: "client_auth",
                usage: "client_auth [bool]",
                description: "This directive specifies whether client certificate \
            authentication (mTLS) is enabled. When true, clients must present a valid \
            certificate. Default: disabled",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            subblock,
        )
        .register(
            Directive {
                name: "client_auth_ca",
                usage: "client_auth_ca <source>",
                description: "This directive specifies the source of trusted CA certificates \
            for verifying client certificates. Supported values: a file path, `system` (OS \
            native root store), or `webpki` (Mozilla root bundle). Default: webpki",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            subblock,
        )
        .register(
            Directive {
                name: "cipher_suite",
                usage: "cipher_suite <name>",
                description: "This directive specifies a cipher suite to add to the allowed \
            list. Repeatable — each occurrence adds one suite.",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            subblock,
        )
        .register(
            Directive {
                name: "ecdh_curve",
                usage: "ecdh_curve <name>",
                description: "This directive specifies an ECDH key exchange group to add to \
            the allowed list, in priority order. Repeatable — each occurrence adds one \
            curve.",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            subblock,
        )
        .register(
            Directive {
                name: "min_version",
                usage: "min_version <version>",
                description: "This directive specifies the minimum allowed TLS version. \
            Supported values: TLSv1.2, TLSv1.3. Default: TLSv1.2",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            subblock,
        )
        .register(
            Directive {
                name: "max_version",
                usage: "max_version <version>",
                description: "This directive specifies the maximum allowed TLS version. \
            Supported values: TLSv1.2, TLSv1.3. Default: TLSv1.3",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            subblock,
        )
        .register(
            Directive {
                name: "ocsp",
                usage: "ocsp { ... }",
                description: "This directive configures OCSP stapling for TLS handshakes.",
                applicable_protocols,
                global_only: false,
                subblock_link: Some(DirectiveSubblock::custom("ocsp")),
            },
            subblock,
        )
        .register(
            Directive {
                name: "ticket_keys",
                usage: "ticket_keys { ... }",
                description: "This directive configures TLS session ticket keys for \
            session resumption.",
                applicable_protocols,
                global_only: false,
                subblock_link: Some(DirectiveSubblock::custom("ticket_keys")),
            },
            subblock,
        );
    register_ocsp_children(registry, applicable_protocols);
    register_ticket_keys_children(registry, applicable_protocols);
}

fn register_ocsp_children(
    _registry: &mut DirectiveRegistry,
    _applicable_protocols: Option<&'static [&'static str]>,
) {
    // Currently, no OCSP children...
}

fn register_ticket_keys_children(
    registry: &mut DirectiveRegistry,
    applicable_protocols: Option<&'static [&'static str]>,
) {
    registry
        .register(
            Directive {
                name: "file",
                usage: "file <path>",
                description: "This directive specifies the file path for persisting TLS \
            session ticket keys across restarts.",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("ticket_keys"),
        )
        .register(
            Directive {
                name: "auto_rotate",
                usage: "auto_rotate [bool]",
                description: "This directive specifies whether TLS session ticket keys \
            should rotate automatically at the configured rotation interval. \
            Default: disabled",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("ticket_keys"),
        )
        .register(
            Directive {
                name: "rotation_interval",
                usage: "rotation_interval <duration>",
                description: "This directive specifies the interval for automatic TLS \
            session ticket key rotation. Default: 12h",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("ticket_keys"),
        )
        .register(
            Directive {
                name: "max_keys",
                usage: "max_keys <count>",
                description: "This directive specifies the maximum number of TLS session \
            ticket keys to keep. Default: 3",
                applicable_protocols,
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("ticket_keys"),
        );
}
