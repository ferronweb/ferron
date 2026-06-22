mod config;
mod formatter;
mod quoting;

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::process;

use clap::Parser as ClapParser;

use config::{FormatConfig, IndentStyle, QuoteStyle};

#[derive(ClapParser)]
#[command(name = "ferron-fmt", about = "A formatter for ferron.conf files")]
struct Cli {
    /// File to format (reads from stdin if not provided)
    file: Option<PathBuf>,

    /// Indentation width
    #[arg(long, default_value_t = 4)]
    indent_width: usize,

    /// Indentation style
    #[arg(long, value_enum, default_value_t = IndentStyle::Spaces)]
    indent_style: IndentStyle,

    /// Quote style for string values
    #[arg(long, value_enum, default_value_t = QuoteStyle::Auto)]
    quote_style: QuoteStyle,

    /// Don't normalize quoting
    #[arg(long)]
    no_normalize_quotes: bool,

    /// Maximum number of consecutive blank lines to preserve
    #[arg(long, default_value_t = 2)]
    max_blank_lines: usize,

    /// Don't add trailing newline
    #[arg(long)]
    no_trailing_newline: bool,

    /// Sort directives alphabetically within blocks
    #[arg(long)]
    sort_directives: bool,

    /// Check if input is already formatted (exit 1 if not)
    #[arg(long)]
    check: bool,

    /// Edit file in place
    #[arg(short, long)]
    in_place: bool,

    /// Write output to file (default: stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();

    let fmt_config = FormatConfig {
        indent_width: cli.indent_width,
        indent_style: cli.indent_style,
        quote_style: cli.quote_style,
        normalize_quotes: !cli.no_normalize_quotes,
        max_blank_lines: cli.max_blank_lines,
        trailing_newline: !cli.no_trailing_newline,
        sort_directives: cli.sort_directives,
    };

    // Read input
    let input = match &cli.file {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading {}: {}", path.display(), e);
                process::exit(1);
            }
        },
        None => {
            let mut buf = String::new();
            if let Err(e) = io::stdin().read_to_string(&mut buf) {
                eprintln!("Error reading stdin: {}", e);
                process::exit(1);
            }
            buf
        }
    };

    // Parse
    let config = match ferronconf::Config::from_str(&input) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            process::exit(1);
        }
    };

    // Format
    let formatted = formatter::format_config(&config, &fmt_config);

    // Check mode
    if cli.check {
        if input == formatted {
            process::exit(0);
        } else {
            process::exit(1);
        }
    }

    // Write output
    if cli.in_place {
        if let Some(path) = &cli.file {
            if let Err(e) = std::fs::write(path, &formatted) {
                eprintln!("Error writing {}: {}", path.display(), e);
                process::exit(1);
            }
        } else {
            eprintln!("--in-place requires a file argument");
            process::exit(1);
        }
    } else if let Some(path) = &cli.output {
        if let Err(e) = std::fs::write(path, &formatted) {
            eprintln!("Error writing {}: {}", path.display(), e);
            process::exit(1);
        }
    } else {
        io::stdout().write_all(formatted.as_bytes()).unwrap();
    }
}
