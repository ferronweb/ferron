use clap::ValueEnum;

/// Configuration for the formatter.
#[derive(Debug, Clone)]
pub struct FormatConfig {
    /// Indentation width (number of spaces or tab width).
    pub indent_width: usize,
    /// Indentation style.
    pub indent_style: IndentStyle,
    /// Quote style for string values.
    pub quote_style: QuoteStyle,
    /// Whether to normalize quoting (use bare when possible).
    pub normalize_quotes: bool,
    /// Maximum number of consecutive blank lines to preserve.
    pub max_blank_lines: usize,
    /// Whether to add a trailing newline.
    pub trailing_newline: bool,
    /// Whether to sort directives alphabetically within blocks.
    pub sort_directives: bool,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            indent_width: 4,
            indent_style: IndentStyle::Spaces,
            quote_style: QuoteStyle::Auto,
            normalize_quotes: true,
            max_blank_lines: 2,
            trailing_newline: true,
            sort_directives: false,
        }
    }
}

/// Indentation style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum IndentStyle {
    /// Use spaces for indentation.
    Spaces,
    /// Use tabs for indentation.
    Tabs,
}

/// Quote style for string values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum QuoteStyle {
    /// Use bare strings when possible, quoted when necessary.
    Auto,
    /// Always use double-quoted strings.
    AlwaysDouble,
    /// Always use bare strings (error if not possible).
    AlwaysBare,
}

impl FormatConfig {
    /// Returns the indentation string for a given nesting depth.
    pub fn indent_at(&self, depth: usize) -> String {
        match self.indent_style {
            IndentStyle::Spaces => " ".repeat(self.indent_width * depth),
            IndentStyle::Tabs => "\t".repeat(depth),
        }
    }
}
