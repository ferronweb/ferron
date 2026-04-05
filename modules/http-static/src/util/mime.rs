//! MIME type detection utilities.

use ferron_core::config::layer::LayeredConfiguration;

/// Get content type for a file path, respecting custom MIME type overrides.
pub fn get_content_type(path: &std::path::Path, config: &LayeredConfiguration) -> Option<String> {
    // Check custom MIME types from config
    for entry in config.get_entries("mime_type", true) {
        if entry.args.len() >= 2 {
            if let (Some(key), Some(val)) = (entry.args[0].as_str(), entry.args[1].as_str()) {
                let ext_match = path
                    .extension()
                    .map(|e| e.to_string_lossy())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if key == ext_match || key == format!(".{ext_match}") {
                    return Some(val.to_string());
                }
            }
        }
    }

    // Fall back to new_mime_guess
    new_mime_guess::from_path(path)
        .first()
        .map(|mime| mime.to_string())
}
