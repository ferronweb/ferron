use std::net::IpAddr;

use cidr::IpCidr;

use crate::HttpContext;

/// Which header to read the client IP from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClientIpHeader {
    /// Read from `X-Forwarded-For` — takes the first (leftmost) IP in the comma-separated chain.
    XForwardedFor,
    /// Read from `Forwarded` (RFC 7239) — parses the first `for=` token.
    Forwarded,
}

impl ClientIpHeader {
    #[inline]
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "x-forwarded-for" => Some(Self::XForwardedFor),
            "forwarded" => Some(Self::Forwarded),
            _ => None,
        }
    }

    #[inline]
    fn header_name(self) -> &'static str {
        match self {
            Self::XForwardedFor => "x-forwarded-for",
            Self::Forwarded => "forwarded",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClientIpFromHeaderConfig {
    header: ClientIpHeader,
    trusted_proxies: Vec<IpCidr>,
}

impl ClientIpFromHeaderConfig {
    /// Extract the client IP from the configured header in the given context.
    ///
    /// Returns `None` if the header is missing or malformed.
    pub fn extract_client_ip(&self, ctx: &HttpContext) -> Option<IpAddr> {
        let header_value = ctx
            .req
            .as_ref()?
            .headers()
            .get(self.header.header_name())?
            .to_str()
            .ok()?;

        let ip = match self.header {
            ClientIpHeader::XForwardedFor => extract_x_forwarded_for(header_value)?,
            ClientIpHeader::Forwarded => extract_forwarded_for(header_value)?,
        };

        Some(ip)
    }

    /// Check if the given IP is in the trusted proxy allowlist.
    pub fn is_trusted_proxy(&self, ip: IpAddr) -> bool {
        if self.trusted_proxies.is_empty() {
            return false;
        }

        let ip = ip.to_canonical();
        self.trusted_proxies.iter().any(|cidr| cidr.contains(&ip))
    }

    /// Resolve which header to use from the configuration. Returns `None` if the
    /// directive is absent or invalid (meaning this stage is a no-op).
    pub fn resolve_from_context(ctx: &HttpContext) -> Option<Self> {
        let entry = ctx.configuration.get_entry("client_ip_from_header", true)?;
        let header_value = entry.args.first()?.as_string_with_interpolations(ctx)?;
        let header = ClientIpHeader::from_str(&header_value)?;
        let trusted_proxies = parse_trusted_proxy_allowlist(entry.children.as_ref(), ctx);

        Some(Self {
            header,
            trusted_proxies,
        })
    }

    /// Returns the name of the header to use for client IP resolution.
    #[inline]
    pub fn header_name(&self) -> &'static str {
        self.header.header_name()
    }
}

/// Extract the client IP from an `X-Forwarded-For` header value.
///
/// `X-Forwarded-For` format: `client, proxy1, proxy2`
/// The leftmost IP is the original client address.
fn extract_x_forwarded_for(value: &str) -> Option<IpAddr> {
    let first = value.split(',').next()?.trim();
    first.parse::<IpAddr>().ok()
}

/// Extract the client IP from a `Forwarded` header value (RFC 7239).
///
/// Format: `for=192.0.2.60;proto=https, for="[2001:db8:ca1::1]:8080";proto=http`
/// We take the first `for=` token from the first forwarded element.
fn extract_forwarded_for(value: &str) -> Option<IpAddr> {
    // Take the first forwarded element
    let first_element = split_forwarded_elements(value).first().copied()?;

    // Find `for=` in the first element
    let for_value = find_forwarded_param(first_element, "for")?;

    // Strip quotes if present
    let unquoted = for_value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(for_value);

    // In RFC 7239, IPv6 addresses are enclosed in brackets: [2001:db8::1]
    // Strip brackets if present (IPv6 literal)
    let cleaned = unquoted
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(unquoted);

    // The `for` value can be an obfuscated identifier like "_hidden" or an IP.
    // We only succeed if it parses as an IP.
    cleaned.parse::<IpAddr>().ok()
}

/// Split a `Forwarded` header value into individual forwarded elements,
/// respecting quoted strings.
fn split_forwarded_elements(value: &str) -> Vec<&str> {
    let mut elements = Vec::new();
    let mut current_start = 0;
    let mut in_quotes = false;

    for (i, ch) in value.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                elements.push(value[current_start..i].trim());
                current_start = i + 1;
            }
            _ => {}
        }
    }

    let remainder = value[current_start..].trim();
    if !remainder.is_empty() {
        elements.push(remainder);
    }

    elements
}

/// Find a parameter value in a forwarded element (e.g. `for=...`, `proto=...`).
fn find_forwarded_param<'a>(element: &'a str, param_name: &str) -> Option<&'a str> {
    let prefix = format!("{param_name}=");

    // Split by `;` to get individual parameters
    for part in element.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix(&prefix) {
            return Some(val.trim());
        }
    }

    None
}

fn parse_trusted_proxy_allowlist(
    children: Option<&ferron_core::config::ServerConfigurationBlock>,
    ctx: &HttpContext,
) -> Vec<IpCidr> {
    let mut trusted_proxies = Vec::new();
    let Some(children) = children else {
        return trusted_proxies;
    };

    if let Some(entries) = children.directives.get("trusted_proxy") {
        for entry in entries {
            for arg in &entry.args {
                if let Some(value) = arg.as_string_with_interpolations(ctx) {
                    if let Ok(cidr) = value.parse::<IpCidr>() {
                        trusted_proxies.push(cidr);
                    }
                }
            }
        }
    }

    trusted_proxies
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper tests ──

    #[test]
    fn client_ip_header_from_str_valid() {
        assert_eq!(
            ClientIpHeader::from_str("x-forwarded-for"),
            Some(ClientIpHeader::XForwardedFor)
        );
        assert_eq!(
            ClientIpHeader::from_str("X-Forwarded-For"),
            Some(ClientIpHeader::XForwardedFor)
        );
        assert_eq!(
            ClientIpHeader::from_str("FORWARDED"),
            Some(ClientIpHeader::Forwarded)
        );
        assert_eq!(
            ClientIpHeader::from_str("forwarded"),
            Some(ClientIpHeader::Forwarded)
        );
    }

    #[test]
    fn client_ip_header_from_str_invalid() {
        assert_eq!(ClientIpHeader::from_str("x-real-ip"), None);
        assert_eq!(ClientIpHeader::from_str("cf-connecting-ip"), None);
        assert_eq!(ClientIpHeader::from_str(""), None);
    }

    #[test]
    fn split_forwarded_elements_single() {
        let elements = split_forwarded_elements("for=192.0.2.60;proto=https");
        assert_eq!(elements, vec!["for=192.0.2.60;proto=https"]);
    }

    #[test]
    fn split_forwarded_elements_multiple() {
        let elements =
            split_forwarded_elements("for=192.0.2.60;proto=https, for=10.0.0.1;proto=http");
        assert_eq!(
            elements,
            vec!["for=192.0.2.60;proto=https", "for=10.0.0.1;proto=http",]
        );
    }

    #[test]
    fn split_forwarded_elements_quoted_comma() {
        let elements =
            split_forwarded_elements("for=\"example.com, inc.\";proto=https, for=10.0.0.1");
        assert_eq!(
            elements,
            vec!["for=\"example.com, inc.\";proto=https", "for=10.0.0.1",]
        );
    }
}
