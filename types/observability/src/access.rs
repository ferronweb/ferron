pub trait AccessEvent: Send + Sync {
    fn protocol(&self) -> &'static str;
    fn visit(&self, visitor: &mut dyn AccessVisitor);

    #[inline]
    fn trace_context(&self) -> Option<&crate::EventTraceContext> {
        None
    }
}

pub trait AccessVisitor {
    fn field_string(&mut self, name: &str, value: &str);
    fn field_u64(&mut self, name: &str, value: u64);
    fn field_f64(&mut self, name: &str, value: f64);
    fn field_bool(&mut self, name: &str, value: bool);
}
