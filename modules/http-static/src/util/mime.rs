//! MIME type detection utilities.

use ferron_core::config::layer::LayeredConfiguration;

/// Get content type for a file path, respecting custom MIME type overrides.
///
/// A `mime_type` mapping whose value cannot be serialized as an HTTP header is
/// skipped, so the lookup falls through to the next mapping and then to the
/// built-in database. The configuration validator rejects such a mapping when
/// Ferron loads the configuration, so skipping it here is a fallback rather
/// than the normal path.
#[inline]
pub fn get_content_type(path: &std::path::Path, config: &LayeredConfiguration) -> Option<String> {
    for entry in config.get_entries("mime_type", false) {
        if entry.args.len() >= 2 {
            if let (Some(key), Some(val)) = (entry.args[0].as_str(), entry.args[1].as_str()) {
                let ext_match = path
                    .extension()
                    .map(|e| e.to_string_lossy())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if key == ext_match || key == format!(".{ext_match}") {
                    if http::HeaderValue::from_str(val).is_ok() {
                        return Some(val.to_string());
                    }
                    ferron_core::log_warn!(
                        "Ignoring `mime_type` mapping for `.{ext_match}`: the value is not a valid \
                         HTTP header value"
                    );
                }
            }
        }
    }

    // Fall back to multi-mime-guess
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy())
        .map(|s| s.to_string())
        .unwrap_or_default();
    multi_mime_guess::lookup(&ext).map(|mime| mime.to_string())
}
