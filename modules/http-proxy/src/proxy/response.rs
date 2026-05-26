//! Response processing logic.

/// Remove headers from the response as indicated by the "Connection" header,
/// per RFC 7230.
pub fn remove_headers_rfc7230(parts: &mut http::response::Parts) {
    if let Some(connection) = parts.headers.get(http::header::CONNECTION) {
        for header in connection
            .to_str()
            .unwrap_or("")
            .split(',')
            .map(|h| h.trim().to_string())
            .collect::<Vec<_>>()
        {
            if header.eq_ignore_ascii_case("upgrade") {
                // Don't break HTTP upgrade handling
                continue;
            }
            parts.headers.remove(header);
        }
        parts.headers.remove(http::header::CONNECTION);
    }
}
