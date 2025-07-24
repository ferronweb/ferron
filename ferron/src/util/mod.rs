mod anti_xss;
#[cfg(feature = "asgi")]
pub mod asgi;
#[cfg(feature = "cache")]
mod atomic_cache;
#[cfg(feature = "replace")]
mod body_replacer;
#[cfg(any(feature = "cgi", feature = "scgi", feature = "fcgi"))]
pub mod cgi;
mod config_macros;
#[cfg(any(feature = "cgi", feature = "scgi", feature = "fcgi"))]
mod copy_move;
mod error_pages;
#[cfg(feature = "fcgi")]
pub mod fcgi;
mod header_placeholders;
mod ip_blocklist;
mod match_hostname;
mod match_location;
mod module_cache;
#[cfg(feature = "runtime-monoio")]
mod monoio_file_stream;
mod no_server_verifier;
#[cfg(feature = "wsgid")]
mod preforked_process_pool;
mod proxy_protocol;
#[cfg(feature = "fcgi")]
mod read_to_end_move;
#[cfg(feature = "runtime-monoio")]
mod send_async_io;
#[cfg(all(feature = "scgi", feature = "runtime-monoio"))]
mod send_read_stream;
#[cfg(feature = "runtime-monoio")]
mod send_rw_stream;
#[cfg(feature = "static")]
mod sizify;
#[cfg(feature = "fcgi")]
mod split_stream_by_map;
mod ttl_cache;
mod url_sanitizer;
#[cfg(any(feature = "wsgi", feature = "wsgid"))]
pub mod wsgi;
#[cfg(feature = "wsgid")]
pub mod wsgid;

pub use anti_xss::*;
#[cfg(feature = "cache")]
pub use atomic_cache::*;
#[cfg(feature = "replace")]
pub use body_replacer::*;
pub(crate) use config_macros::*;
#[cfg(any(feature = "cgi", feature = "scgi", feature = "fcgi"))]
pub use copy_move::*;
pub use error_pages::*;
pub use header_placeholders::*;
pub use ip_blocklist::*;
pub use match_hostname::*;
pub use match_location::*;
pub use module_cache::*;
#[cfg(feature = "runtime-monoio")]
pub use monoio_file_stream::*;
pub use no_server_verifier::*;
#[cfg(feature = "wsgid")]
pub use preforked_process_pool::*;
pub use proxy_protocol::*;
#[cfg(feature = "fcgi")]
pub use read_to_end_move::*;
#[cfg(feature = "runtime-monoio")]
pub use send_async_io::*;
#[cfg(all(feature = "scgi", feature = "runtime-monoio"))]
pub use send_read_stream::*;
#[cfg(feature = "runtime-monoio")]
pub use send_rw_stream::*;
#[cfg(feature = "static")]
pub use sizify::*;
#[cfg(feature = "fcgi")]
pub use split_stream_by_map::*;
pub use ttl_cache::*;
pub use url_sanitizer::*;

/// The web server software identifier
pub const SERVER_SOFTWARE: &str = "Ferron";
