use std::collections::BTreeMap;

/// A structured access log event.
///
/// Implement this trait to provide protocol-specific access log fields
/// (HTTP, DNS, TLS, etc.) to access log sinks. The visitor pattern
/// allows sinks to consume fields without allocating a `HashMap`.
pub trait AccessEvent: Send + Sync {
    /// The protocol name (e.g. `"http"`, `"dns"`).
    fn protocol(&self) -> &'static str;
    /// Visit each field of this access event using the visitor pattern.
    fn visit(&self, visitor: &mut dyn AccessVisitor);

    /// W3C trace context attached to this access event, if any.
    #[inline]
    fn trace_context(&self) -> Option<&crate::EventTraceContext> {
        None
    }

    /// The time at which the event occurred. Used by OTLP `log_style modern` to
    /// set the log record timestamp. Defaults to `None` (the SDK uses the
    /// current time when unset).
    #[inline]
    fn event_time(&self) -> Option<std::time::SystemTime> {
        None
    }

    /// Control plane metadata to include as `ferron.control_plane.*` attributes.
    #[inline]
    fn control_plane_metadata(&self) -> Option<&BTreeMap<String, String>> {
        None
    }
}

/// A visitor that receives individual access log fields.
///
/// Sinks implement this trait to consume access log fields without
/// allocating intermediate data structures.
pub trait AccessVisitor {
    /// Record a string field.
    fn field_string(&mut self, name: &str, value: &str);
    /// Record an unsigned integer field.
    fn field_u64(&mut self, name: &str, value: u64);
    /// Record a floating-point field.
    fn field_f64(&mut self, name: &str, value: f64);
    /// Record a boolean field.
    fn field_bool(&mut self, name: &str, value: bool);
}
