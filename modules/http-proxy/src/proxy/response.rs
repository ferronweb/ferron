//! Response processing logic.

/// Remove headers from the response as indicated by the "Connection" header,
/// per RFC 7230.
#[inline]
pub fn remove_headers_rfc7230(parts: &mut http::response::Parts) {
    // Clone the HeaderValue to break the borrow on parts.headers,
    // allowing the mutable remove calls inside the loop.
    let connection_value = parts.headers.get(http::header::CONNECTION).cloned();

    if let Some(connection) = connection_value {
        let connection_str = connection.to_str().unwrap_or("");
        for header in connection_str.split(',').map(|h| h.trim()) {
            if !header.eq_ignore_ascii_case("upgrade") {
                parts.headers.remove(header);
            }
        }
        parts.headers.remove(http::header::CONNECTION);
    }
}
