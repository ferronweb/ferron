pub trait ConfigurationValidator {
    /// Validates the provided configuration block.
    fn validate_block(
        &self,
        config: &crate::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
