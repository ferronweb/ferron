#![no_main]

use http::StatusCode;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    if input.len() < 2 {
        return;
    }

    let status_code_raw = u16::from_le_bytes([input[0], input[1]]);
    let status_code = StatusCode::from_u16(status_code_raw % 600).unwrap_or(StatusCode::OK);

    let email = if input.len() > 2 {
        core::str::from_utf8(&input[2..]).ok()
    } else {
        None
    };

    // Generate error page — must not panic for any status code or email
    let page =
        ferron_http_server::util::error_pages::generate_default_error_page(status_code, email);

    // Invariant: output must be valid HTML
    assert!(
        page.contains("<!doctype html>") || page.contains("<!DOCTYPE html>"),
        "error page must contain doctype"
    );
    assert!(
        page.contains("</html>"),
        "error page must contain closing </html> tag"
    );

    // Invariant: status code must appear in the output
    assert!(
        page.contains(&status_code.as_u16().to_string()),
        "error page must contain the status code number"
    );

    // Invariant: email must be XSS-safe (if provided)
    if let Some(email) = email {
        if !email.is_empty() {
            assert!(
                !page.contains(&format!("{}>", email)),
                "email must be escaped in HTML output (found unescaped email)"
            );
        }
    }
});
