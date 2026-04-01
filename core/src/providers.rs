pub trait Provider<C>: Send + Sync {
    /// Returns the name of this provider
    fn name(&self) -> &str;

    /// Execute the provider with the given context
    fn execute(&self, ctx: &mut C) -> Result<(), Box<dyn std::error::Error>>;
}
