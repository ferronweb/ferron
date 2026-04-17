#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AcmeCause {
    DnsNxDomain,
    DnsServFail,
    Timeout,
    ConnectionRefused,
    ConnectionReset,
    TlsHandshakeFailed,
    Http4xx,
    Http5xx,
    InvalidResponse,
    #[default]
    Unknown,
}

impl std::fmt::Display for AcmeCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcmeCause::DnsNxDomain => write!(f, "DNS NXDOMAIN"),
            AcmeCause::DnsServFail => write!(f, "DNS SERVFAIL"),
            AcmeCause::Timeout => write!(f, "timeout"),
            AcmeCause::ConnectionRefused => write!(f, "connection refused"),
            AcmeCause::ConnectionReset => write!(f, "connection reset"),
            AcmeCause::TlsHandshakeFailed => write!(f, "TLS handshake failed"),
            AcmeCause::Http4xx => write!(f, "HTTP 4xx"),
            AcmeCause::Http5xx => write!(f, "HTTP 5xx"),
            AcmeCause::InvalidResponse => write!(f, "invalid response"),
            AcmeCause::Unknown => write!(f, "unknown (see details)"),
        }
    }
}

pub fn parse_acme_cause(detail: &str) -> AcmeCause {
    let s = detail.to_lowercase();

    // --- DNS errors (most specific first) ---
    if s.contains("no such host") || s.contains("nxdomain") {
        return AcmeCause::DnsNxDomain;
    }
    if s.contains("servfail") || s.contains("server misbehaving") {
        return AcmeCause::DnsServFail;
    }

    // --- Connection / network errors ---
    if s.contains("timeout") || s.contains("timed out") || s.contains("deadline exceeded") {
        return AcmeCause::Timeout;
    }
    if s.contains("connection refused") {
        return AcmeCause::ConnectionRefused;
    }
    if s.contains("connection reset") || s.contains("reset by peer") {
        return AcmeCause::ConnectionReset;
    }

    // --- TLS errors ---
    if s.contains("tls")
        || s.contains("handshake failure")
        || s.contains("certificate verify failed")
    {
        return AcmeCause::TlsHandshakeFailed;
    }

    // --- HTTP response errors ---
    // Simple heuristic: look for common status codes in text
    if contains_http_status(&s, 400..=499) {
        return AcmeCause::Http4xx;
    }
    if contains_http_status(&s, 500..=599) {
        return AcmeCause::Http5xx;
    }

    // --- ACME semantic / validation errors ---
    if s.contains("did not match")
        || s.contains("invalid response")
        || s.contains("key authorization")
    {
        return AcmeCause::InvalidResponse;
    }

    AcmeCause::Unknown
}

// Helper: detect HTTP status codes in text without heavy parsing
fn contains_http_status(s: &str, range: std::ops::RangeInclusive<u16>) -> bool {
    for code in range {
        // match " 404 ", ":404", "(404)", etc.
        let code_str = code.to_string();
        if s.contains(&format!(" {}", code_str))
            || s.contains(&format!(":{}", code_str))
            || s.contains(&format!("({})", code_str))
        {
            return true;
        }
    }
    false
}

pub fn acme_error_to_string(e: &instant_acme::Error) -> String {
    match e {
        instant_acme::Error::Api(e) => {
            format!("{}\nFull error: {e}", parse_acme_cause(&e.to_string()))
        }
        e => e.to_string(),
    }
}
