use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use ferron_core::{
    config::{
        adapter::ConfigurationAdapter, ServerConfiguration, ServerConfigurationBlock,
        ServerConfigurationDirectiveEntry, ServerConfigurationHostFilters,
        ServerConfigurationInterpolatedStringPart, ServerConfigurationMatcher,
        ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
        ServerConfigurationMatcherOperator, ServerConfigurationPort, ServerConfigurationSpan,
        ServerConfigurationValue,
    },
    loader::ModuleLoader,
};
use ferronconf::{
    Block, Config, Directive, HostLabels, MatchBlock, Operand, Operator, SnippetBlock, Statement,
    StringPart, Value,
};

struct FerronConfConfigurationAdapter;

impl FerronConfConfigurationAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ConfigurationAdapter for FerronConfConfigurationAdapter {
    fn adapt(
        &self,
        params: &HashMap<String, String>,
    ) -> Result<
        (
            ServerConfiguration,
            Box<dyn ferron_core::config::adapter::ConfigurationWatcher>,
        ),
        Box<dyn std::error::Error>,
    > {
        let filename = params.get("file").ok_or(anyhow!(
            "'file' parameter is required for 'ferronconf' configuration adapter"
        ))?;

        let mut include_stack = Vec::new();
        let mut loaded_files = Vec::new();
        let statements =
            load_top_level_statements(Path::new(filename), &mut include_stack, &mut loaded_files)?;

        Ok((
            translate_configuration(&statements)?,
            Box::new(FerronConfConfigurationWatcher {
                _files: loaded_files,
            }),
        ))
    }

    fn file_extension(&self) -> Vec<&'static str> {
        vec!["conf"]
    }
}

#[derive(Debug, Clone)]
struct SourceStatement {
    statement: Statement,
    file: PathBuf,
}

#[derive(Debug, Clone)]
struct SnippetDefinition {
    block: Block,
    file: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct TranslationScope {
    matchers: HashMap<String, ServerConfigurationMatcher>,
    snippets: HashMap<String, SnippetDefinition>,
}

impl TranslationScope {
    fn extend(&self, local: TranslationScope) -> Self {
        let mut matchers = self.matchers.clone();
        matchers.extend(local.matchers);

        let mut snippets = self.snippets.clone();
        snippets.extend(local.snippets);

        Self { matchers, snippets }
    }
}

#[derive(Debug, Clone, Default)]
struct MergedBlock {
    directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    matchers: HashMap<String, ServerConfigurationMatcher>,
    span: Option<ServerConfigurationSpan>,
}

impl MergedBlock {
    fn new(span: Option<ServerConfigurationSpan>) -> Self {
        Self {
            directives: HashMap::new(),
            matchers: HashMap::new(),
            span,
        }
    }

    fn push_directive(
        &mut self,
        name: impl Into<String>,
        entry: ServerConfigurationDirectiveEntry,
    ) {
        self.directives.entry(name.into()).or_default().push(entry);
    }

    fn merge(&mut self, other: Self) {
        for (name, mut entries) in other.directives {
            self.directives
                .entry(name)
                .or_default()
                .append(&mut entries);
        }

        self.matchers.extend(other.matchers);

        if self.span.is_none() {
            self.span = other.span;
        }
    }

    fn into_block(self) -> ServerConfigurationBlock {
        ServerConfigurationBlock {
            directives: Arc::new(self.directives),
            matchers: self.matchers,
            span: self.span,
        }
    }
}

impl From<ServerConfigurationBlock> for MergedBlock {
    fn from(block: ServerConfigurationBlock) -> Self {
        Self {
            directives: Arc::try_unwrap(block.directives).unwrap_or_else(|arc| (*arc).clone()),
            matchers: block.matchers,
            span: block.span,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct HostMergeKey {
    protocol: String,
    port: Option<u16>,
    ip: Option<IpAddr>,
    host: Option<String>,
}

fn load_top_level_statements(
    path: &Path,
    include_stack: &mut Vec<PathBuf>,
    loaded_files: &mut Vec<PathBuf>,
) -> anyhow::Result<Vec<SourceStatement>> {
    let path = fs::canonicalize(path)
        .with_context(|| format!("Failed to resolve configuration file '{}'", path.display()))?;

    if let Some(existing) = include_stack.iter().position(|included| included == &path) {
        let mut cycle = include_stack[existing..]
            .iter()
            .map(|entry| entry.display().to_string())
            .collect::<Vec<_>>();
        cycle.push(path.display().to_string());
        anyhow::bail!("Include cycle detected: {}", cycle.join(" -> "));
    }

    if !loaded_files.iter().any(|loaded| loaded == &path) {
        loaded_files.push(path.clone());
    }

    let source = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read configuration file '{}'", path.display()))?;
    let config = Config::from_str(&source)
        .with_context(|| format!("Failed to parse configuration file '{}'", path.display()))?;

    include_stack.push(path.clone());

    let mut statements = Vec::new();
    for statement in config.statements {
        match statement {
            Statement::Directive(directive) if directive.name == "include" => {
                let include_path = extract_top_level_include_path(&directive, &path)?;
                let include_path = if include_path.is_absolute() {
                    include_path
                } else {
                    path.parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(include_path)
                };

                statements.extend(load_top_level_statements(
                    &include_path,
                    include_stack,
                    loaded_files,
                )?);

                if let Some(block) = directive.block {
                    statements.push(SourceStatement {
                        statement: Statement::GlobalBlock(block),
                        file: path.clone(),
                    });
                }
            }
            statement => statements.push(SourceStatement {
                statement,
                file: path.clone(),
            }),
        }
    }

    include_stack.pop();

    Ok(statements)
}

fn extract_top_level_include_path(directive: &Directive, file: &Path) -> anyhow::Result<PathBuf> {
    if directive.args.len() != 1 {
        anyhow::bail!(
            "Top-level include directives require exactly one string argument ({})",
            format_source_location(file, directive.span.line, directive.span.column)
        );
    }

    Ok(PathBuf::from(value_to_plain_string(
        &directive.args[0],
        file,
        "Top-level include directives",
    )?))
}

fn translate_configuration(statements: &[SourceStatement]) -> anyhow::Result<ServerConfiguration> {
    let top_scope = collect_top_level_scope(statements)?;
    let mut global = MergedBlock::default();
    global.matchers = top_scope.matchers.clone();

    let mut hosts = BTreeMap::<HostMergeKey, MergedBlock>::new();

    for statement in statements {
        match &statement.statement {
            Statement::Directive(directive) => {
                merge_directive_into_block(
                    directive,
                    &statement.file,
                    &top_scope,
                    &mut global,
                    &mut Vec::new(),
                )?;
            }
            Statement::GlobalBlock(block) => {
                global.merge(MergedBlock::from(translate_block(
                    block,
                    &statement.file,
                    &top_scope,
                    &mut Vec::new(),
                )?));
            }
            Statement::HostBlock(host_block) => {
                let host_config = translate_block(
                    &host_block.block,
                    &statement.file,
                    &top_scope,
                    &mut Vec::new(),
                )?;
                let mut seen_keys = BTreeSet::new();

                for host_pattern in &host_block.hosts {
                    let (protocol, filters, port) = translate_host_pattern(host_pattern);
                    let key = HostMergeKey {
                        protocol,
                        port,
                        ip: filters.ip,
                        host: filters.host,
                    };

                    if !seen_keys.insert(key.clone()) {
                        continue;
                    }

                    match hosts.entry(key) {
                        std::collections::btree_map::Entry::Occupied(mut entry) => {
                            entry
                                .get_mut()
                                .merge(MergedBlock::from(host_config.clone()));
                        }
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(MergedBlock::from(host_config.clone()));
                        }
                    }
                }
            }
            Statement::MatchBlock(_) | Statement::SnippetBlock(_) => {}
        }
    }

    let mut grouped_ports: BTreeMap<
        String,
        BTreeMap<Option<u16>, Vec<(ServerConfigurationHostFilters, ServerConfigurationBlock)>>,
    > = BTreeMap::new();

    for (key, block) in hosts {
        grouped_ports
            .entry(key.protocol)
            .or_default()
            .entry(key.port)
            .or_default()
            .push((
                ServerConfigurationHostFilters {
                    ip: key.ip,
                    host: key.host,
                },
                block.into_block(),
            ));
    }

    let ports = grouped_ports
        .into_iter()
        .map(|(protocol, port_groups)| {
            let ports = port_groups
                .into_iter()
                .map(|(port, hosts)| ServerConfigurationPort { port, hosts })
                .collect::<Vec<_>>();
            (protocol, ports)
        })
        .collect::<BTreeMap<_, _>>();

    Ok(ServerConfiguration {
        global_config: Arc::new(global.into_block()),
        ports,
    })
}

fn collect_top_level_scope(statements: &[SourceStatement]) -> anyhow::Result<TranslationScope> {
    let mut scope = TranslationScope::default();

    for statement in statements {
        collect_global_scope_from_statement(&statement.statement, &statement.file, &mut scope)?;
    }

    Ok(scope)
}

fn collect_global_scope_from_statement(
    statement: &Statement,
    file: &Path,
    scope: &mut TranslationScope,
) -> anyhow::Result<()> {
    match statement {
        Statement::MatchBlock(match_block) => register_matcher(scope, match_block, file)?,
        Statement::SnippetBlock(snippet_block) => register_snippet(scope, snippet_block, file)?,
        Statement::GlobalBlock(block) => {
            for nested in &block.statements {
                collect_global_scope_from_statement(nested, file, scope)?;
            }
        }
        Statement::Directive(_) | Statement::HostBlock(_) => {}
    }

    Ok(())
}

fn collect_block_scope(statements: &[Statement], file: &Path) -> anyhow::Result<TranslationScope> {
    let mut scope = TranslationScope::default();

    for statement in statements {
        match statement {
            Statement::MatchBlock(match_block) => register_matcher(&mut scope, match_block, file)?,
            Statement::SnippetBlock(snippet_block) => {
                register_snippet(&mut scope, snippet_block, file)?
            }
            Statement::Directive(_) | Statement::HostBlock(_) | Statement::GlobalBlock(_) => {}
        }
    }

    Ok(scope)
}

fn register_matcher(
    scope: &mut TranslationScope,
    match_block: &MatchBlock,
    file: &Path,
) -> anyhow::Result<()> {
    let matcher = translate_match_block(match_block, file)?;
    if scope
        .matchers
        .insert(match_block.matcher.clone(), matcher)
        .is_some()
    {
        anyhow::bail!(
            "Duplicate matcher '{}' ({})",
            match_block.matcher,
            format_source_location(file, match_block.span.line, match_block.span.column)
        );
    }

    Ok(())
}

fn register_snippet(
    scope: &mut TranslationScope,
    snippet_block: &SnippetBlock,
    file: &Path,
) -> anyhow::Result<()> {
    let snippet = SnippetDefinition {
        block: snippet_block.block.clone(),
        file: file.to_path_buf(),
    };

    if scope
        .snippets
        .insert(snippet_block.name.clone(), snippet)
        .is_some()
    {
        anyhow::bail!(
            "Duplicate snippet '{}' ({})",
            snippet_block.name,
            format_source_location(file, snippet_block.span.line, snippet_block.span.column)
        );
    }

    Ok(())
}

fn translate_block(
    block: &Block,
    file: &Path,
    inherited_scope: &TranslationScope,
    snippet_stack: &mut Vec<String>,
) -> anyhow::Result<ServerConfigurationBlock> {
    let local_scope = collect_block_scope(&block.statements, file)?;
    let scope = inherited_scope.extend(local_scope);
    let mut translated =
        MergedBlock::new(Some(span_from(file, block.span.line, block.span.column)));
    translated.matchers = scope.matchers.clone();

    for statement in &block.statements {
        match statement {
            Statement::Directive(directive) => {
                merge_directive_into_block(
                    directive,
                    file,
                    &scope,
                    &mut translated,
                    snippet_stack,
                )?;
            }
            Statement::GlobalBlock(nested) => {
                translated.merge(MergedBlock::from(translate_block(
                    nested,
                    file,
                    &scope,
                    snippet_stack,
                )?));
            }
            Statement::MatchBlock(_) | Statement::SnippetBlock(_) => {}
            Statement::HostBlock(_) => {
                anyhow::bail!(
                    "Host blocks are only supported at the top level ({})",
                    format_source_location(file, block.span.line, block.span.column)
                );
            }
        }
    }

    Ok(translated.into_block())
}

fn merge_directive_into_block(
    directive: &Directive,
    file: &Path,
    scope: &TranslationScope,
    block: &mut MergedBlock,
    snippet_stack: &mut Vec<String>,
) -> anyhow::Result<()> {
    if let Some(snippet) = resolve_snippet_directive(directive, scope, file)? {
        if snippet_stack.iter().any(|name| name == &snippet.0) {
            let mut cycle = snippet_stack.clone();
            cycle.push(snippet.0.clone());
            anyhow::bail!("Snippet cycle detected: {}", cycle.join(" -> "));
        }

        snippet_stack.push(snippet.0);
        block.merge(MergedBlock::from(translate_block(
            &snippet.1.block,
            &snippet.1.file,
            scope,
            snippet_stack,
        )?));
        snippet_stack.pop();
        return Ok(());
    }

    let children = directive
        .block
        .as_ref()
        .map(|child| translate_block(child, file, scope, snippet_stack))
        .transpose()?;

    let entry = ServerConfigurationDirectiveEntry {
        args: directive
            .args
            .iter()
            .map(|value| translate_value(value, file))
            .collect(),
        children,
        span: Some(span_from(file, directive.span.line, directive.span.column)),
    };

    block.push_directive(directive.name.clone(), entry);

    Ok(())
}

fn resolve_snippet_directive(
    directive: &Directive,
    scope: &TranslationScope,
    file: &Path,
) -> anyhow::Result<Option<(String, SnippetDefinition)>> {
    if directive.block.is_some() {
        return Ok(None);
    }

    if directive.name != "include" && directive.name != "use" {
        return Ok(None);
    }

    if directive.args.len() != 1 {
        return Ok(None);
    }

    let snippet_name = value_to_plain_string(&directive.args[0], file, "Snippet references")?;

    Ok(scope
        .snippets
        .get(&snippet_name)
        .cloned()
        .map(|snippet| (snippet_name, snippet)))
}

fn translate_host_pattern(
    host_pattern: &ferronconf::HostPattern,
) -> (String, ServerConfigurationHostFilters, Option<u16>) {
    let protocol = host_pattern
        .protocol
        .clone()
        .unwrap_or_else(|| "http".to_string());

    let filters = match &host_pattern.labels {
        HostLabels::IpAddr(ip) => ServerConfigurationHostFilters {
            ip: Some(*ip),
            host: None,
        },
        HostLabels::Hostname(labels) => ServerConfigurationHostFilters {
            ip: None,
            host: Some(labels.join(".")),
        },
        HostLabels::Wildcard => ServerConfigurationHostFilters::default(),
    };

    (protocol, filters, host_pattern.port)
}

fn translate_match_block(
    match_block: &MatchBlock,
    file: &Path,
) -> anyhow::Result<ServerConfigurationMatcher> {
    Ok(ServerConfigurationMatcher {
        exprs: match_block
            .expr
            .iter()
            .map(|expr| translate_match_expression(expr, file))
            .collect::<anyhow::Result<Vec<_>>>()?,
        span: Some(span_from(
            file,
            match_block.span.line,
            match_block.span.column,
        )),
    })
}

fn translate_match_expression(
    expression: &ferronconf::MatcherExpression,
    file: &Path,
) -> anyhow::Result<ServerConfigurationMatcherExpr> {
    Ok(ServerConfigurationMatcherExpr {
        left: translate_operand(&expression.left, file)?,
        right: translate_operand(&expression.right, file)?,
        op: match expression.op {
            Operator::Eq => ServerConfigurationMatcherOperator::Eq,
            Operator::NotEq => ServerConfigurationMatcherOperator::NotEq,
            Operator::Regex => ServerConfigurationMatcherOperator::Regex,
            Operator::NotRegex => ServerConfigurationMatcherOperator::NotRegex,
            Operator::In => ServerConfigurationMatcherOperator::In,
        },
    })
}

fn translate_operand(
    operand: &Operand,
    _file: &Path,
) -> anyhow::Result<ServerConfigurationMatcherOperand> {
    Ok(match operand {
        Operand::Identifier(parts, _) => {
            ServerConfigurationMatcherOperand::Identifier(parts.join("."))
        }
        Operand::String(value, _) => ServerConfigurationMatcherOperand::String(value.clone()),
        Operand::Integer(value, _) => ServerConfigurationMatcherOperand::Integer(*value),
        Operand::Float(value, _) => ServerConfigurationMatcherOperand::Float(*value),
    })
}

fn translate_value(value: &Value, file: &Path) -> ServerConfigurationValue {
    match value {
        Value::String(value, span) => ServerConfigurationValue::String(
            value.clone(),
            Some(span_from(file, span.line, span.column)),
        ),
        Value::Integer(value, span) => {
            ServerConfigurationValue::Number(*value, Some(span_from(file, span.line, span.column)))
        }
        Value::Float(value, span) => {
            ServerConfigurationValue::Float(*value, Some(span_from(file, span.line, span.column)))
        }
        Value::Boolean(value, span) => {
            ServerConfigurationValue::Boolean(*value, Some(span_from(file, span.line, span.column)))
        }
        Value::InterpolatedString(parts, span) => ServerConfigurationValue::InterpolatedString(
            parts.iter().map(translate_string_part).collect(),
            Some(span_from(file, span.line, span.column)),
        ),
    }
}

fn translate_string_part(part: &StringPart) -> ServerConfigurationInterpolatedStringPart {
    match part {
        StringPart::Literal(value) => {
            ServerConfigurationInterpolatedStringPart::String(value.clone())
        }
        StringPart::Expression(path) => {
            ServerConfigurationInterpolatedStringPart::Variable(path.join("."))
        }
    }
}

fn value_to_plain_string(value: &Value, file: &Path, context: &str) -> anyhow::Result<String> {
    match value {
        Value::String(value, _) => Ok(value.clone()),
        Value::InterpolatedString(parts, _span)
            if parts
                .iter()
                .all(|part| matches!(part, StringPart::Literal(_))) =>
        {
            let value = parts
                .iter()
                .map(|part| match part {
                    StringPart::Literal(literal) => literal.as_str(),
                    StringPart::Expression(_) => unreachable!(),
                })
                .collect::<String>();
            Ok(value)
        }
        _ => anyhow::bail!(
            "{} must use a plain string literal ({})",
            context,
            format_source_location(file, value.span().line, value.span().column)
        ),
    }
}

fn span_from(file: &Path, line: usize, column: usize) -> ServerConfigurationSpan {
    ServerConfigurationSpan {
        line,
        column,
        file: Some(file.display().to_string()),
    }
}

fn format_source_location(file: &Path, line: usize, column: usize) -> String {
    format!(
        "file '{}' at line {}, column {}",
        file.display(),
        line,
        column
    )
}

struct FerronConfConfigurationWatcher {
    _files: Vec<PathBuf>,
}

#[async_trait]
impl ferron_core::config::adapter::ConfigurationWatcher for FerronConfConfigurationWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        std::future::pending().await
    }
}

pub struct FerronConfConfigurationAdapterModuleLoader;

impl ModuleLoader for FerronConfConfigurationAdapterModuleLoader {
    fn register_configuration_adapters(
        &mut self,
        registry: &mut HashMap<&'static str, Box<dyn ConfigurationAdapter>>,
    ) {
        registry.insert(
            "ferronconf",
            Box::new(FerronConfConfigurationAdapter::new()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("invalid clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ferron-config-ferronconf-{unique}-{}",
                std::process::id()
            ));

            fs::create_dir_all(&path).expect("failed to create test directory");

            Self { path }
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path.join(name);
            fs::write(&path, contents).expect("failed to write configuration file");
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn adapt_file(path: &Path) -> ServerConfiguration {
        let mut params = HashMap::new();
        params.insert("file".to_string(), path.display().to_string());

        FerronConfConfigurationAdapter::new()
            .adapt(&params)
            .expect("configuration should adapt successfully")
            .0
    }

    #[test]
    fn adapt_loads_includes_and_merges_global_and_host_blocks() {
        let dir = TestDir::new();
        let shared = dir.write(
            "shared.conf",
            r#"
{
    runtime {
        workers 4
    }
}

match is_cli {
    request.header.user_agent ~ "curl"
}

example.com {
    header X-Shared 1
}
"#,
        );

        let main = dir.write(
            "main.conf",
            r#"
include "shared.conf"

{
    runtime {
        io_uring true
    }
}

match is_api {
    request.path ~ "^/api"
}

example.com {
    root /srv/www
    if is_api {
        header X-Api 1
    }
}

http example.com {
    header X-Explicit 1
}

example.com {
    header X-Test 2
}

http example.com:8080 {
    header X-Port 8080
}
"#,
        );

        let config = adapt_file(&main);
        let shared_path = shared.display().to_string();

        let runtime = config
            .global_config
            .directives
            .get("runtime")
            .expect("runtime directives should exist");
        assert_eq!(runtime.len(), 2);
        assert_eq!(
            runtime[0]
                .span
                .as_ref()
                .and_then(|span| span.file.as_deref()),
            Some(shared_path.as_str())
        );

        let http_ports = config.ports.get("http").expect("http ports should exist");
        let default_port = http_ports
            .iter()
            .find(|port| port.port.is_none())
            .expect("default http port should exist");
        assert_eq!(default_port.hosts.len(), 1);

        let (filters, block) = &default_port.hosts[0];
        assert_eq!(filters.host.as_deref(), Some("example.com"));
        assert_eq!(block.directives.get("root").map(Vec::len), Some(1));
        assert_eq!(block.directives.get("header").map(Vec::len), Some(3));
        assert!(block.matchers.contains_key("is_api"));
        assert!(block.matchers.contains_key("is_cli"));

        let if_entry = block
            .directives
            .get("if")
            .and_then(|entries| entries.first())
            .expect("if directive should exist");
        let if_block = if_entry
            .children
            .as_ref()
            .expect("if directive should have a child block");
        assert!(if_block.matchers.contains_key("is_api"));
        assert!(if_block.matchers.contains_key("is_cli"));

        let port_8080 = http_ports
            .iter()
            .find(|port| port.port == Some(8080))
            .expect("http:8080 config should exist");
        assert_eq!(port_8080.hosts.len(), 1);
        assert_eq!(port_8080.hosts[0].0.host.as_deref(), Some("example.com"));
    }

    #[test]
    fn adapt_expands_snippets_inside_blocks() {
        let dir = TestDir::new();
        dir.write(
            "shared.conf",
            r#"
snippet shared_defaults {
    header X-Shared 1
}
"#,
        );

        let main = dir.write(
            "main.conf",
            r#"
include "shared.conf"

snippet local_defaults {
    header X-Local 1
}

example.com {
    include shared_defaults
    use local_defaults
    header X-Direct 1
}
"#,
        );

        let config = adapt_file(&main);

        let http_ports = config.ports.get("http").expect("http ports should exist");
        let host = &http_ports
            .iter()
            .find(|port| port.port.is_none())
            .expect("default http port should exist")
            .hosts[0]
            .1;

        assert_eq!(host.directives.get("header").map(Vec::len), Some(3));
    }

    #[test]
    fn adapt_rejects_include_cycles() {
        let dir = TestDir::new();
        let main = dir.write("main.conf", "include \"other.conf\"\n");
        dir.write("other.conf", "include \"main.conf\"\n");

        let mut params = HashMap::new();
        params.insert("file".to_string(), main.display().to_string());

        let result = FerronConfConfigurationAdapter::new().adapt(&params);
        assert!(result.is_err(), "cyclic includes should fail");
        let error = result.err().expect("result should contain an error");

        assert!(error.to_string().contains("Include cycle detected"));
    }
}
