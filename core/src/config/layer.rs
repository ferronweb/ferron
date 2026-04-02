use std::sync::Arc;

#[derive(Clone, Default)]
pub struct LayeredConfiguration {
    pub layers: Vec<Arc<crate::config::ServerConfigurationBlock>>,
}

impl LayeredConfiguration {
    #[inline]
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    #[inline]
    pub fn add_layer(&mut self, layer: Arc<crate::config::ServerConfigurationBlock>) {
        self.layers.push(layer);
    }

    #[inline]
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

    #[inline]
    pub fn get_entry<'a>(
        &'a self,
        directive: &str,
        inherit: bool,
    ) -> Option<&'a crate::config::ServerConfigurationDirectiveEntry> {
        let mut entries = self.get_entries(directive, inherit);
        entries.pop()
    }

    #[inline]
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

    #[inline]
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
