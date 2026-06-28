//! Configuration validator for the HTTP static file module

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
    ) -> Result<(), Box<dyn std::error::Error>> {
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
                ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::String(_, _)
            ], {});
        }

        if let Some(entries) = config.directives.get("file_cache_control") {
            if let Some(entry) = entries.first() {
                if let Some(ServerConfigurationValue::String(val, span)) = entry.args.first() {
                    if val.contains('\r') || val.contains('\n') || val.contains('\0') {
                        ctx.add_best_practice_violation(
                            "`file_cache_control` value contains invalid HTTP header characters (\\r, \\n, or \\0)",
                            span.clone(),
                        );
                    }
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
