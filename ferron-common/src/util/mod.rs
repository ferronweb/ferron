mod anti_xss;
mod config_macros;
mod default_html_page;
mod header_placeholders;
mod ip_blocklist;
mod is_localhost;
mod match_hostname;
mod match_location;
mod module_cache;
mod no_server_verifier;
mod sizify;
mod ttl_cache;

pub use anti_xss::*;
pub use header_placeholders::*;
pub use ip_blocklist::*;
pub use is_localhost::*;
pub use match_hostname::*;
pub use match_location::*;
pub use module_cache::*;
pub use no_server_verifier::*;
pub use sizify::*;
pub use ttl_cache::*;

/// The web server software identifier
pub const SERVER_SOFTWARE: &str = "Ferron";
