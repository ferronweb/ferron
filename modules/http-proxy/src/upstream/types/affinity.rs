//! Affinity types for session stickiness.

use http::header::HeaderName;
use std::time::Duration;

/// SameSite cookie attribute mode.
#[derive(Clone, Copy, Debug, Default)]
pub enum SameSiteMode {
    Strict,
    #[default]
    Lax,
    None,
}

impl SameSiteMode {
    /// Return the string representation for Set-Cookie header.
    pub fn as_str(&self) -> &'static str {
        match self {
            SameSiteMode::Strict => "Strict",
            SameSiteMode::Lax => "Lax",
            SameSiteMode::None => "None",
        }
    }
}

/// Cookie-based session affinity configuration.
#[derive(Clone, Debug)]
pub struct CookieAffinityConfig {
    /// Cookie name.
    pub name: String,
    /// Cookie TTL (None = session cookie).
    pub ttl: Option<Duration>,
    /// Cookie path.
    pub path: String,
    /// Optional cookie domain.
    pub domain: Option<String>,
    /// Whether to set the Secure flag.
    pub secure: bool,
    /// Whether to set the HttpOnly flag.
    pub httponly: bool,
    /// SameSite attribute.
    pub samesite: SameSiteMode,
}

impl Default for CookieAffinityConfig {
    fn default() -> Self {
        Self {
            name: "ferron_sticky".to_string(),
            ttl: None,
            path: "/".to_string(),
            domain: None,
            secure: false,
            httponly: true,
            samesite: SameSiteMode::Lax,
        }
    }
}

/// Hash method for hash-based affinity.
#[derive(Clone, Copy, Debug, Default)]
pub enum HashMethod {
    #[default]
    Consistent,
    Modulus,
}

/// Session affinity type configuration.
#[derive(Clone, Debug)]
pub enum AffinityType {
    /// Cookie-based affinity.
    Cookie(CookieAffinityConfig),
    /// Header-based affinity.
    Header(HeaderName),
    /// Client IP-based affinity.
    Ip,
    /// Hash-based affinity with configurable variable.
    Hash {
        /// Variable to hash (e.g. `"request.header.x-session-id"`).
        variable: String,
        /// Hash method to use.
        #[allow(dead_code)]
        method: HashMethod,
    },
}

/// Session affinity configuration.
#[derive(Clone, Debug)]
pub struct AffinityConfig {
    /// The type of affinity to use.
    pub affinity_type: AffinityType,
}
