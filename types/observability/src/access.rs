pub trait AccessEvent: Send + Sync {
    fn protocol(&self) -> &'static str;
    fn visit(&self, visitor: &mut dyn AccessVisitor);

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
}

pub trait AccessVisitor {
    fn field_string(&mut self, name: &str, value: &str);
    fn field_u64(&mut self, name: &str, value: u64);
    fn field_f64(&mut self, name: &str, value: f64);
    fn field_bool(&mut self, name: &str, value: bool);
}
