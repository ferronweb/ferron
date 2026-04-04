//! HTTP context types

#[cfg(feature = "util")]
pub mod util;
pub mod variables;

use std::collections::HashMap;
use std::net::SocketAddr;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::Variables;
use ferron_observability::CompositeEventSink;
use http::{HeaderMap, Request, Response, Uri};
use http_body_util::combinators::UnsyncBoxBody;
use typemap_rev::{TypeMap, TypeMapKey};

pub type HttpRequest = Request<UnsyncBoxBody<bytes::Bytes, std::io::Error>>;
pub enum HttpResponse {
    Custom(Response<UnsyncBoxBody<bytes::Bytes, std::io::Error>>),
    BuiltinError(u16, Option<HeaderMap>),
    Abort,
}

pub struct HttpContext {
    pub req: Option<HttpRequest>,
    pub res: Option<HttpResponse>,
    pub events: CompositeEventSink,
    pub configuration: LayeredConfiguration,
    pub hostname: Option<String>,
    pub variables: HashMap<String, String>,
    pub previous_error: Option<u16>,
    pub original_uri: Option<Uri>,
    pub encrypted: bool,
    pub local_address: SocketAddr,
    pub remote_address: SocketAddr,
    pub auth_user: Option<String>,
    pub extensions: TypeMap,
}

impl Variables for HttpContext {
    fn resolve(&self, key: &str) -> Option<String> {
        if let Some(req) = &self.req {
            variables::resolve_variable(key, req, &self.variables)
        } else {
            self.variables.resolve(key)
        }
    }
}

impl HttpContext {
    /// Insert a value into the extensions type map.
    ///
    /// If a value of this type already exists, it will be replaced.
    pub fn insert<T: TypeMapKey>(&mut self, value: T::Value) {
        self.extensions.insert::<T>(value);
    }

    /// Get a reference to a value from the extensions type map.
    pub fn get<T: TypeMapKey>(&self) -> Option<&T::Value> {
        self.extensions.get::<T>()
    }

    /// Get a mutable reference to a value from the extensions type map.
    pub fn get_mut<T: TypeMapKey>(&mut self) -> Option<&mut T::Value> {
        self.extensions.get_mut::<T>()
    }

    /// Remove a value from the extensions type map and return it.
    pub fn remove<T: TypeMapKey>(&mut self) -> Option<T::Value> {
        self.extensions.remove::<T>()
    }

    /// Check if a value of the given type exists in the extensions type map.
    pub fn contains<T: TypeMapKey>(&self) -> bool {
        self.extensions.contains_key::<T>()
    }
}

pub struct HttpFileContext {
    pub http: HttpContext,
    pub metadata: vibeio::fs::Metadata,
    pub file_path: std::path::PathBuf,
    pub path_info: Option<String>, // For example, "/test" in "/index.php/test"
    pub file_root: std::path::PathBuf,
}

pub struct HttpErrorContext {
    pub error_code: u16,
    pub headers: Option<HeaderMap>,
    pub configuration: LayeredConfiguration,
    pub res: Option<Response<UnsyncBoxBody<bytes::Bytes, std::io::Error>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Type map key definitions
    struct UserIdKey;
    impl TypeMapKey for UserIdKey {
        type Value = u64;
    }

    struct UserNameKey;
    impl TypeMapKey for UserNameKey {
        type Value = String;
    }

    #[test]
    fn test_extensions_insert_and_get() {
        let mut ctx = create_test_context();

        ctx.insert::<UserIdKey>(42);
        ctx.insert::<UserNameKey>("test_user".to_string());

        assert_eq!(ctx.get::<UserIdKey>(), Some(&42));
        assert_eq!(ctx.get::<UserNameKey>(), Some(&"test_user".to_string()));
    }

    #[test]
    fn test_extensions_insert_replaces_existing() {
        let mut ctx = create_test_context();

        ctx.insert::<UserIdKey>(42);
        assert_eq!(ctx.get::<UserIdKey>(), Some(&42));

        ctx.insert::<UserIdKey>(100);
        assert_eq!(ctx.get::<UserIdKey>(), Some(&100));
    }

    #[test]
    fn test_extensions_get_mut() {
        let mut ctx = create_test_context();

        ctx.insert::<UserNameKey>("initial".to_string());

        if let Some(name) = ctx.get_mut::<UserNameKey>() {
            name.push_str("_modified");
        }

        assert_eq!(
            ctx.get::<UserNameKey>(),
            Some(&"initial_modified".to_string())
        );
    }

    #[test]
    fn test_extensions_remove() {
        let mut ctx = create_test_context();

        ctx.insert::<UserIdKey>(42);
        assert_eq!(ctx.get::<UserIdKey>(), Some(&42));

        let removed = ctx.remove::<UserIdKey>();
        assert_eq!(removed, Some(42));
        assert_eq!(ctx.get::<UserIdKey>(), None);
    }

    #[test]
    fn test_extensions_contains() {
        let mut ctx = create_test_context();

        assert!(!ctx.contains::<UserIdKey>());

        ctx.insert::<UserIdKey>(42);
        assert!(ctx.contains::<UserIdKey>());
    }

    #[test]
    fn test_extensions_different_types_independent() {
        let mut ctx = create_test_context();

        ctx.insert::<UserIdKey>(42);
        assert!(!ctx.contains::<UserNameKey>());

        ctx.insert::<UserNameKey>("user".to_string());
        assert!(ctx.contains::<UserIdKey>());
        assert!(ctx.contains::<UserNameKey>());
    }

    fn create_test_context() -> HttpContext {
        HttpContext {
            req: None,
            res: None,
            events: ferron_observability::CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname: None,
            variables: HashMap::new(),
            previous_error: None,
            original_uri: None,
            encrypted: false,
            local_address: "127.0.0.1:8080".parse().unwrap(),
            remote_address: "127.0.0.1:12345".parse().unwrap(),
            auth_user: None,
            extensions: TypeMap::new(),
        }
    }
}
