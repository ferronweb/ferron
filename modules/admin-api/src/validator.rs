//! Admin configuration validator.
//!
//! Validates the `admin { ... }` global configuration directive.

use std::collections::HashSet;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationValue;

pub struct AdminConfigurationValidator;

impl ConfigurationValidator for AdminConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check for the admin directive
        let Some(admin_entries) = config.directives.get("admin") else {
            return Ok(());
        };
        used_directives.insert("admin".to_string());

        for admin_entry in admin_entries {
            let Some(admin_block) = &admin_entry.children else {
                return Err("Invalid directive 'admin': missing nested block"
                    .to_string()
                    .into());
            };

            // Validate listen
            if let Some(listen_entries) = admin_block.directives.get("listen") {
                used_directives.insert("listen".to_string());
                for entry in listen_entries {
                    if entry.args.len() > 1 {
                        return Err(format!(
                            "Invalid directive 'admin.listen': expected at most 1 argument(s), got {}",
                            entry.args.len()
                        )
                        .into());
                    }
                    if let Some(arg) = entry.args.first() {
                        if !matches!(
                            arg,
                            ServerConfigurationValue::String(_, _)
                                | ServerConfigurationValue::InterpolatedString(_, _)
                        ) {
                            return Err(
                                "Invalid directive 'admin.listen': argument type mismatch".into()
                            );
                        }
                    }
                }
            }

            // Validate endpoint flag directives: health, status, config, reload
            for directive_name in &["health", "status", "config", "reload"] {
                if let Some(entries) = admin_block.directives.get(*directive_name) {
                    used_directives.insert(directive_name.to_string());
                    for entry in entries {
                        if entry.args.len() > 1 {
                            return Err(format!(
                                "Invalid directive 'admin.{}': expected at most 1 argument(s), got {}",
                                directive_name,
                                entry.args.len()
                            )
                            .into());
                        }
                        if let Some(arg) = entry.args.first() {
                            if !matches!(
                                arg,
                                ServerConfigurationValue::Boolean(_, _)
                                    | ServerConfigurationValue::String(_, _)
                            ) {
                                return Err(format!(
                                    "Invalid directive 'admin.{}': argument type mismatch",
                                    directive_name
                                )
                                .into());
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
