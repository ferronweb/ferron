//! Configuration validator for the HTTP static file module

use ferron_core::config::validator::ConfigurationValidationError;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};
use ferron_core::validate_directive;

pub struct HttpStaticConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for HttpStaticConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        {
            let used_directives = &mut ctx.used_directives;
            // Static file compression (on-the-fly)
            validate_directive!(config, used_directives, compressed, optional
                args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

            // Precompressed file serving
            validate_directive!(config, used_directives, precompressed, optional
                args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

            // ETag generation
            validate_directive!(config, used_directives, etag, optional
                args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

            // Directory listing
            validate_directive!(config, used_directives, directory_listing, optional
                args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

            // Cache-Control header for static files
            validate_directive!(config, used_directives, file_cache_control, optional
                args(1) => [
                    ServerConfigurationValue::String(_, _)
                        | ServerConfigurationValue::InterpolatedString(_, _)
                        | ServerConfigurationValue::Boolean(false, _)
                ], {});

            // Custom MIME type mappings
            validate_directive!(config, used_directives, mime_type, optional
                args(2) => [
                    ServerConfigurationValue::String(_, _),
                    ServerConfigurationValue::String(_, _)
                ], {});

            // Custom error pages (status codes followed by file path)
            // Format: error_page <code1> [code2 ...] <file_path>
            // Minimum 2 args enforced at runtime in ErrorPageStage
            validate_directive!(config, used_directives, error_page, optional args(*) => [
                ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
            ], {});

            // Error page placeholder substitution
            validate_directive!(config, used_directives, error_page_placeholders, optional
                args(1) => [ServerConfigurationValue::Boolean(_, _)], {});
        }

        if let Some(entries) = config.directives.get("file_cache_control") {
            if let Some(entry) = entries.first() {
                if let Some(ServerConfigurationValue::String(val, span)) = entry.args.first() {
                    check_header_value("`file_cache_control`", val, span.clone(), "Cache-Control")?;
                }
            }
        }

        // A `mime_type` value is sent as a `Content-Type` response header and is
        // written verbatim into `multipart/byteranges` part headers, so a value
        // that cannot be serialized as a header would otherwise break every
        // response for the matching extension.
        if let Some(entries) = config.directives.get("mime_type") {
            for entry in entries {
                if let Some(ServerConfigurationValue::String(val, span)) = entry.args.get(1) {
                    check_header_value("`mime_type`", val, span.clone(), "Content-Type")?;
                }
            }
        }

        if first_flag(config, "directory_listing") == Some(true) {
            ctx.add_best_practice_violation(
                "`directory_listing` exposes generated indexes for directories without index files; enable it only for intentionally public file listings",
                first_entry_span(config, "directory_listing"),
            );
        }

        Ok(())
    }
}

/// Report a directive value that cannot be sent as the given response header.
///
/// The value reaches a response header verbatim, and the HTTP crate rejects
/// control bytes and line breaks. Treat that as a configuration error so the
/// server refuses to start instead of failing every matching request.
fn check_header_value(
    directive: &str,
    value: &str,
    span: Option<ServerConfigurationSpan>,
    header_name: &str,
) -> Result<(), ConfigurationValidationError> {
    if http::HeaderValue::from_str(value).is_ok() {
        return Ok(());
    }
    Err(ConfigurationValidationError::from(format!(
        "{directive} value is not a valid {header_name} header value; it must not contain \
             control bytes or line breaks"
    ))
    .with_span(span))
}

fn first_flag(block: &ServerConfigurationBlock, directive: &str) -> Option<bool> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .map(ServerConfigurationDirectiveEntry::get_flag)
}

fn first_entry_span(
    block: &ServerConfigurationBlock,
    directive: &str,
) -> Option<ServerConfigurationSpan> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .and_then(entry_span)
}

fn entry_span(entry: &ServerConfigurationDirectiveEntry) -> Option<ServerConfigurationSpan> {
    entry.span.clone().or_else(|| {
        entry.args.first().and_then(|value| match value {
            ServerConfigurationValue::String(_, span)
            | ServerConfigurationValue::Number(_, span)
            | ServerConfigurationValue::Float(_, span)
            | ServerConfigurationValue::Boolean(_, span)
            | ServerConfigurationValue::InterpolatedString(_, span) => span.clone(),
        })
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::arc_with_non_send_sync)]
    use super::*;
    use ferron_core::config::validator::{
        ConfigurationValidationError, ConfigurationValidator, ConfigurationValidatorContext,
    };

    fn entry(args: Vec<ServerConfigurationValue>) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args,
            children: None,
            span: None,
        }
    }

    fn string(value: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(value.to_string(), None)
    }

    fn validate(block: &ServerConfigurationBlock) -> Vec<String> {
        let mut ctx = ConfigurationValidatorContext {
            used_directives: std::collections::HashSet::new(),
            is_global: false,
            scoped_validators: std::sync::Arc::new(std::collections::HashMap::new()),
            diagnostics: Vec::new(),
            scope: None,
        };
        HttpStaticConfigurationValidator
            .validate_block(block, &mut ctx)
            .expect("validation should not fail structurally");
        ctx.diagnostics.into_iter().map(|d| d.message).collect()
    }

    fn block(
        directive: &str,
        arg_lists: Vec<Vec<ServerConfigurationValue>>,
    ) -> ServerConfigurationBlock {
        let mut block = ServerConfigurationBlock::default();
        let map = std::sync::Arc::make_mut(&mut block.directives);
        map.insert(
            directive.to_string(),
            arg_lists.into_iter().map(entry).collect(),
        );
        block
    }

    fn mime_type_block(entries: &[(&str, &str)]) -> ServerConfigurationBlock {
        let args: Vec<Vec<ServerConfigurationValue>> = entries
            .iter()
            .map(|(ext, value)| vec![string(ext), string(value)])
            .collect();
        block("mime_type", args)
    }

    fn cache_control_block(value: &str) -> ServerConfigurationBlock {
        block("file_cache_control", vec![vec![string(value)]])
    }

    #[test]
    fn mime_type_rejects_header_injection() {
        let messages = validate(&mime_type_block(&[(
            ".txt",
            "text/plain\r\nX-Injected: yes",
        )]));
        assert!(
            messages.iter().any(|m| m.contains("`mime_type`")),
            "expected a `mime_type` error, got {messages:?}"
        );
    }

    #[test]
    fn mime_type_rejects_control_bytes() {
        for value in ["text/plain\u{1}", "text/plain\u{7f}", "text/plain\u{0}"] {
            let messages = validate(&mime_type_block(&[(".txt", value)]));
            assert!(
                messages.iter().any(|m| m.contains("`mime_type`")),
                "expected `{value}` to be rejected, got {messages:?}"
            );
        }
    }

    #[test]
    fn mime_type_accepts_ordinary_values() {
        for value in [
            "text/plain",
            "text/plain; charset=utf-8",
            "application/wasm",
        ] {
            assert!(
                validate(&mime_type_block(&[(".txt", value)])).is_empty(),
                "expected `{value}` to be accepted"
            );
        }
    }

    #[test]
    fn mime_type_checks_every_entry() {
        let messages = validate(&mime_type_block(&[
            (".txt", "text/plain"),
            (".bin", "application/x\u{1}"),
        ]));
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.contains("`mime_type`"))
                .count(),
            1,
            "only the invalid entry should be reported, got {messages:?}"
        );
    }

    #[test]
    fn file_cache_control_rejects_header_injection() {
        for value in ["public\u{1}", "public\r\nX-Injected: yes", "public\u{7f}"] {
            let messages = validate(&cache_control_block(value));
            assert!(
                messages.iter().any(|m| m.contains("`file_cache_control`")),
                "expected `{value}` to be rejected, got {messages:?}"
            );
        }
    }

    #[test]
    fn file_cache_control_accepts_ordinary_values() {
        assert!(validate(&cache_control_block("public, max-age=3600")).is_empty());
    }

    #[test]
    fn empty_block_validates() {
        let mut ctx = ConfigurationValidatorContext {
            used_directives: std::collections::HashSet::new(),
            is_global: false,
            scoped_validators: std::sync::Arc::new(std::collections::HashMap::new()),
            diagnostics: Vec::new(),
            scope: None,
        };
        let result: Result<(), ConfigurationValidationError> = HttpStaticConfigurationValidator
            .validate_block(&ServerConfigurationBlock::default(), &mut ctx);
        assert!(result.is_ok());
    }
}
