//! Display implementations for formatting AST nodes as `ferron.conf` source code.
//!
//! This module provides [`fmt::Display`](std::fmt::Display) implementations for all
//! AST types, allowing configuration to be serialized back to text format.
//!
//! # Example
//!
//! ```rust
//! # use ferronconf::Config;
//! # use std::str::FromStr;
//! let config = Config::from_str("example.com { root /var/www }").unwrap();
//! println!("{}", config); // Prints the formatted configuration
//! ```

use indenter::indented;

use std::fmt;

use super::*;

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for stmt in &self.statements {
            if first {
                first = false;
            } else {
                write!(f, "\n\n")?;
            }
            write!(f, "{}", stmt)?;
        }
        Ok(())
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Statement::Directive(d) => write!(f, "{}", d),
            Statement::HostBlock(h) => write!(f, "{}", h),
            Statement::MatchBlock(m) => write!(f, "{}", m),
            Statement::GlobalBlock(g) => write!(f, "{}", g),
            Statement::SnippetBlock(s) => write!(f, "{}", s),
        }
    }
}

impl fmt::Display for Directive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)?;
        for arg in &self.args {
            write!(f, " {}", arg)?;
        }
        if let Some(block) = &self.block {
            write!(f, " {{\n{}}}", block)?;
        }
        Ok(())
    }
}

impl fmt::Display for HostBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, host) in self.hosts.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", host)?;
        }
        write!(f, " {{\n{}}}", self.block)
    }
}

impl fmt::Display for MatchBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "match {} {{", self.matcher)?;
        let mut indented = indented(f).with_format(indenter::Format::Uniform { indentation: "  " });
        for expr in &self.expr {
            writeln!(indented, "{}", expr)?;
        }
        write!(f, "}}")
    }
}

impl fmt::Display for SnippetBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "snippet {} {}", self.name, self.block)
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut indented = indented(f).with_format(indenter::Format::Uniform { indentation: "  " });
        for stmt in &self.statements {
            writeln!(indented, "{}", stmt)?;
        }
        Ok(())
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::String(s, _) => write!(f, "{:?}", s),
            Value::Integer(i, _) => write!(f, "{}", i),
            Value::Float(fl, _) => write!(f, "{}", fl),
            Value::Boolean(b, _) => write!(f, "{}", b),
            Value::InterpolatedString(is, _) => {
                write!(f, "\"")?;
                for part in is {
                    match part {
                        StringPart::Literal(s) => write!(f, "{}", s.escape_debug())?,
                        StringPart::Expression(v) => {
                            let mut expr = String::new();
                            expr.push_str("{{");
                            expr.push_str(&v.join("."));
                            expr.push_str("}}");
                            write!(f, "{}", expr)?;
                        }
                    }
                }
                write!(f, "\"")?;
                Ok(())
            }
        }
    }
}

impl fmt::Display for HostLabels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hostname(hostname) => write!(f, "{}", hostname.join(".")),
            Self::IpAddr(IpAddr::V4(v4)) => write!(f, "{}", v4),
            Self::IpAddr(IpAddr::V6(v6)) => write!(f, "[{}]", v6),
            Self::Wildcard => write!(f, "*"),
        }
    }
}

impl fmt::Display for HostPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(protocol) = &self.protocol {
            write!(f, "{} ", protocol)?;
        }
        write!(f, "{}", self.labels)?;
        if let Some(port) = self.port {
            write!(f, ":{}", port)?;
        }
        Ok(())
    }
}

impl fmt::Display for MatcherExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.left, self.op.as_str(), self.right)
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Identifier(parts, _) => write!(f, "{}", parts.join(".")),
            Operand::String(s, _) => write!(f, "{:?}", s),
            Operand::Integer(i, _) => write!(f, "{}", i),
            Operand::Float(fl, _) => write!(f, "{}", fl),
        }
    }
}
