/// Escapes some characters as HTML entities, preventing XSS vulnerabilities
pub fn anti_xss(input: &str) -> String {
  input
    .replace("&", "&amp;")
    .replace("<", "&lt;")
    .replace(">", "&gt;")
    .replace("\"", "&quot;")
}
