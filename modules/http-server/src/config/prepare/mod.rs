use std::{collections::HashMap, net::IpAddr, sync::Arc};

use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationMatcherExpr,
};

/// Named and default host configurations for a given IP scope.
///
/// Separating the default host from named hosts enables:
/// - Zero-allocation lookups via `HashMap::get(hostname)` using `&str` (Borrow)
/// - Cheap `Arc::clone` instead of deep cloning on every request
#[derive(Debug, Clone, Default)]
pub struct HostConfigs {
    /// Default host config (for `host None` / `_` block)
    pub default_host: Option<Arc<PreparedHostConfigurationBlock>>,
    /// Named host configs, keyed by hostname
    pub named_hosts: HashMap<String, Arc<PreparedHostConfigurationBlock>>,
}

impl HostConfigs {
    #[cfg(test)]
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a host configuration (None hostname = default)
    #[inline]
    pub fn insert(
        &mut self,
        hostname: Option<String>,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        match hostname {
            Some(name) => {
                self.named_hosts.insert(name, config);
            }
            None => {
                self.default_host = Some(config);
            }
        }
    }

    /// Look up a host configuration by hostname.
    /// Falls back to the default host if no named match is found.
    #[allow(dead_code)]
    #[inline]
    pub fn get(&self, hostname: &str) -> Option<&Arc<PreparedHostConfigurationBlock>> {
        self.named_hosts
            .get(hostname)
            .or(self.default_host.as_ref())
    }

    /// Get the default host configuration, if any
    #[allow(dead_code)]
    #[inline]
    pub fn get_default(&self) -> Option<&Arc<PreparedHostConfigurationBlock>> {
        self.default_host.as_ref()
    }
}

pub type PreparedConfiguration = HashMap<Option<IpAddr>, HostConfigs>;

#[derive(Debug, Clone)]
pub struct PreparedHostConfigurationBlock {
    pub directives: Arc<std::collections::HashMap<String, Vec<ServerConfigurationDirectiveEntry>>>,
    pub matches: Vec<PreparedHostConfigurationMatch>,
    pub error_config: Vec<PreparedHostConfigurationErrorConfig>,
}

impl TryFrom<ServerConfigurationBlock> for PreparedHostConfigurationBlock {
    type Error = Box<dyn std::error::Error>;

    #[inline]
    fn try_from(value: ServerConfigurationBlock) -> Result<Self, Self::Error> {
        prepare_host_block(value)
    }
}

#[derive(Debug, Clone)]
pub struct PreparedHostConfigurationMatch {
    pub matcher: PreparedHostConfigurationMatcher,
    pub config: Arc<PreparedHostConfigurationBlock>,
}

#[derive(Eq, PartialEq, Ord, PartialOrd, Debug, Clone)]
pub enum PreparedHostConfigurationMatcher {
    Location(String),
    IfConditional(Vec<ServerConfigurationMatcherExpr>),
    IfNotConditional(Vec<ServerConfigurationMatcherExpr>),
}

#[derive(Debug, Clone)]
pub struct PreparedHostConfigurationErrorConfig {
    pub error_code: Option<u16>,
    pub config: PreparedHostConfigurationBlock,
}

#[inline]
pub fn prepare_host_config(
    port: ferron_core::config::ServerConfigurationPort,
) -> Result<PreparedConfiguration, Box<dyn std::error::Error>> {
    let mut result: PreparedConfiguration = PreparedConfiguration::new();
    for host in port.hosts {
        let ip = host.0.ip;
        let hostname = host.0.host;
        let config = host.1;

        let prepared_config = Arc::new(prepare_host_block(config)?);

        result
            .entry(ip)
            .or_default()
            .insert(hostname, prepared_config);
    }
    Ok(result)
}

#[inline]
pub fn prepare_host_block(
    config: ferron_core::config::ServerConfigurationBlock,
) -> Result<PreparedHostConfigurationBlock, Box<dyn std::error::Error>> {
    // Unwrap the Arc or clone if shared
    let mut directives = Arc::try_unwrap(config.directives).unwrap_or_else(|arc| (*arc).clone());

    let mut block = PreparedHostConfigurationBlock {
        directives: Arc::new(HashMap::new()), // Placeholder, will be set at the end
        matches: Vec::new(),
        error_config: Vec::new(),
    };

    // Matches (locations)
    if let Some(entries) = directives.remove("location") {
        for entry in entries {
            if let Some(ferron_core::config::ServerConfigurationValue::String(location, _)) =
                entry.args.first()
            {
                let matches_one = PreparedHostConfigurationMatch {
                    matcher: PreparedHostConfigurationMatcher::Location(location.clone()),
                    config: Arc::new(prepare_host_block(
                        entry
                            .children
                            .ok_or(anyhow::anyhow!("Location directive must have a block"))?,
                    )?),
                };

                if let Some(matches) = block.matches.iter_mut().find(|m| {
                    matches!(
                        m.matcher,
                        PreparedHostConfigurationMatcher::Location(ref loc) if loc == location
                    )
                }) {
                    // Merge duplicate location blocks
                    let mut new_directives = (*matches.config.directives).clone();
                    for (k, v) in matches_one.config.directives.iter() {
                        new_directives
                            .entry(k.clone())
                            .or_insert_with(Vec::new)
                            .extend(v.iter().cloned());
                    }
                    let mut matches_config = (*matches.config).clone();
                    matches_config
                        .matches
                        .extend(matches_one.config.matches.clone());
                    matches_config
                        .error_config
                        .extend(matches_one.config.error_config.clone());
                    matches_config.directives = Arc::new(new_directives);
                    matches.config = Arc::new(matches_config);
                } else {
                    block.matches.push(matches_one);
                }
            }
        }
    }

    // Matches (if conditional)
    if let Some(entries) = directives.remove("if") {
        for entry in entries {
            if let Some(ferron_core::config::ServerConfigurationValue::String(matcher, _)) =
                entry.args.first()
            {
                let matches_one = PreparedHostConfigurationMatch {
                    matcher: PreparedHostConfigurationMatcher::IfConditional(
                        config
                            .matchers
                            .get(matcher)
                            .cloned()
                            .ok_or(anyhow::anyhow!("Undefined matcher '{}'", matcher))?
                            .exprs,
                    ),
                    config: Arc::new(prepare_host_block(
                        entry
                            .children
                            .ok_or(anyhow::anyhow!("Location directive must have a block"))?,
                    )?),
                };

                if let Some(matches) = block
                    .matches
                    .iter_mut()
                    .find(|m| matches!(m.matcher, ref cond if cond == &matches_one.matcher))
                {
                    // Merge duplicate blocks
                    let mut new_directives = (*matches.config.directives).clone();
                    for (k, v) in matches_one.config.directives.iter() {
                        new_directives
                            .entry(k.clone())
                            .or_insert_with(Vec::new)
                            .extend(v.iter().cloned());
                    }
                    let mut matches_config = (*matches.config).clone();
                    matches_config
                        .matches
                        .extend(matches_one.config.matches.clone());
                    matches_config
                        .error_config
                        .extend(matches_one.config.error_config.clone());
                    matches_config.directives = Arc::new(new_directives);
                    matches.config = Arc::new(matches_config);
                } else {
                    block.matches.push(matches_one);
                }
            }
        }
    }

    // Matches (if_not conditional)
    if let Some(entries) = directives.remove("if_not") {
        for entry in entries {
            if let Some(ferron_core::config::ServerConfigurationValue::String(matcher, _)) =
                entry.args.first()
            {
                let matches_one = PreparedHostConfigurationMatch {
                    matcher: PreparedHostConfigurationMatcher::IfNotConditional(
                        config
                            .matchers
                            .get(matcher)
                            .cloned()
                            .ok_or(anyhow::anyhow!("Undefined matcher '{}'", matcher))?
                            .exprs,
                    ),
                    config: Arc::new(prepare_host_block(
                        entry
                            .children
                            .ok_or(anyhow::anyhow!("Location directive must have a block"))?,
                    )?),
                };

                if let Some(matches) = block
                    .matches
                    .iter_mut()
                    .find(|m| matches!(m.matcher, ref cond if cond == &matches_one.matcher))
                {
                    // Merge duplicate blocks
                    let mut new_directives = (*matches.config.directives).clone();
                    for (k, v) in matches_one.config.directives.iter() {
                        new_directives
                            .entry(k.clone())
                            .or_insert_with(Vec::new)
                            .extend(v.iter().cloned());
                    }
                    let mut matches_config = (*matches.config).clone();
                    matches_config
                        .matches
                        .extend(matches_one.config.matches.clone());
                    matches_config
                        .error_config
                        .extend(matches_one.config.error_config.clone());
                    matches_config.directives = Arc::new(new_directives);
                    matches.config = Arc::new(matches_config);
                } else {
                    block.matches.push(matches_one);
                }
            }
        }
    }

    // Error configs
    if let Some(entries) = directives.remove("handle_error") {
        for entry in entries {
            let error_code = entry.args.first().and_then(|arg| {
                if let ferron_core::config::ServerConfigurationValue::Number(code, _) = arg {
                    Some(*code as u16)
                } else {
                    None
                }
            });
            let error_config = PreparedHostConfigurationErrorConfig {
                error_code,
                config: prepare_host_block(
                    entry
                        .children
                        .ok_or(anyhow::anyhow!("Error directive must have a block"))?,
                )?,
            };
            if let Some(existing) = block
                .error_config
                .iter_mut()
                .find(|e| e.error_code == error_code)
            {
                // Merge duplicate error configs
                let mut new_directives = (*existing.config.directives).clone();
                for (k, v) in error_config.config.directives.iter() {
                    new_directives
                        .entry(k.clone())
                        .or_insert_with(Vec::new)
                        .extend(v.iter().cloned());
                }
                existing.config.matches.extend(error_config.config.matches);
                existing
                    .config
                    .error_config
                    .extend(error_config.config.error_config);
                existing.config.directives = Arc::new(new_directives);
            } else {
                block.error_config.push(error_config);
            }
        }
    }

    // Set the final directives (wrapped in Arc)
    block.directives = Arc::new(directives);

    Ok(block)
}

#[cfg(test)]
mod tests;
