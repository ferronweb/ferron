//! HTML entity escaping to prevent cross-site scripting (XSS).

/// Escape special characters as HTML entities to prevent XSS vulnerabilities.
///
/// Converts `&`, `<`, `>`, `"`, and `'` to their HTML entity equivalents.
pub fn anti_xss(input: &str) -> String {
    input
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
        .replace("'", "&#x27;")
}
