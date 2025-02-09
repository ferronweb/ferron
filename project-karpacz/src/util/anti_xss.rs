pub fn anti_xss(input: &str) -> String {
  input
    .replace("&", "&amp;")
    .replace("<", "&gt;")
    .replace(">", "&lt;")
    .replace("\"", "&quot;")
}
