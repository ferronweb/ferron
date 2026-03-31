use std::sync::Arc;

pub struct LayeredConfiguration {
    pub layers: Vec<Arc<crate::config::ServerConfigurationBlock>>,
}

impl LayeredConfiguration {
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    pub fn add_layer(&mut self, layer: Arc<crate::config::ServerConfigurationBlock>) {
        self.layers.push(layer);
    }

    pub fn get_entries<'a>(
        &'a self,
        directive: &str,
        inherit: bool,
    ) -> Vec<&'a crate::config::ServerConfigurationDirectiveEntry> {
        let mut entries = Vec::new();
        for layer in self.layers.iter().rev() {
            if let Some(directives) = layer.directives.get(directive) {
                entries.extend(directives);
            }
            if !inherit {
                break;
            }
        }
        entries
    }

    pub fn get_entry<'a>(
        &'a self,
        directive: &str,
        inherit: bool,
    ) -> Option<&'a crate::config::ServerConfigurationDirectiveEntry> {
        let mut entries = self.get_entries(directive, inherit);
        entries.pop()
    }

    pub fn get_value(
        &self,
        directive: &str,
        inherit: bool,
    ) -> Option<&crate::config::ServerConfigurationValue> {
        if let Some(entry) = self.get_entry(directive, inherit) {
            entry.args.first()
        } else {
            None
        }
    }

    pub fn get_flag(&self, directive: &str, inherit: bool) -> bool {
        if let Some(entry) = self.get_entry(directive, inherit) {
            if let Some(crate::config::ServerConfigurationValue::Boolean(value, _)) =
                entry.args.first()
            {
                return *value;
            } else {
                return true;
            }
        }
        false
    }
}
