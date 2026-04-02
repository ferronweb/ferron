# `ferron.conf` file format specification (v1.0)

## 1. Overview

The `ferron.conf` format is a domain-specific configuration language designed for custom web server configurations. It supports directives, host-based blocks, match conditions, and reusable snippets.

This specification defines the formal syntax of the format based on its EBNF grammar and the reference Rust implementation.

## 2. Lexical structure

### 2.1 Character set

The configuration file is encoded in UTF-8 and contains:
- **Alphabetic characters** - `A-Z`, `a-z`
- **Numeric digits** - `0-9`
- **Special symbols** - `{ } [ ] : . * , - = ! ~ / + _ " \ #`

### 2.2 Whitespace and comments

- **Whitespace** (spaces, tabs, newlines) is discarded by the lexer except where syntactically significant.
- **Comments** begin with `#` and extend to the end of the line.
- Comments are only recognized between statements; they are not allowed inside `match` blocks.

### 2.3 Tokens

| Token Type | Description | Examples |
|------------|-------------|----------|
| `Identifier` | Alphanumeric sequence starting with a letter | `server_name`, `max_connections` |
| `Number` | Integer or decimal value | `80`, `443`, `1.5` |
| `StringQuoted` | Double-quoted string (supports escapes) | `"example.com"`, `"path/to/file"` |
| `StringBare` | Unquoted string of valid characters | `localhost`, `index.html` |
| `Boolean` | Literal values `true` or `false` | `true`, `false` |
| `Interpolation` | Variable interpolation syntax | `${variable}`, `{{path.to.value}}` |

## 3. Syntax grammar

### 3.1 Top-level structure

```ebnf
config          ::= statement* EOF

statement       ::= directive
                  | host-block
                  | match-block
                  | global-block
                  | snippet-block
```

A configuration file consists of zero or more statements at the top level.

## 4. Statement types

### 4.1 Directives

Directives define configuration parameters with optional values and blocks:

```ebnf
directive       ::= identifier value* block?
value           ::= string | number | boolean | interpolation
block           ::= '{' statement* '}'
interpolation   ::= '{{' identifier-path '}}'
identifier-path ::= identifier ( '.' identifier )*
```

**Examples:**
```ferron
server_name example.com
max_connections 1000
enabled true
root "{{app.root}}"
```

### 4.2 Host blocks

Host blocks apply configuration rules to specific hosts:

```ebnf
host-block      ::= host-pattern ( ',' host-pattern )* block
host-pattern    ::= protocol? host ( ':' port )?
protocol        ::= identifier | bare-string
host            ::= '*' | hostname | ipv4 | '[' ipv6 ']'
hostname        ::= host-label ( '.' host-label )*
host-label      ::= identifier | '*'
ipv4            ::= dec-octet '.' dec-octet '.' dec-octet '.' dec-octet
dec-octet       ::= DIGIT+  /* validated as 0–255 */
ipv6            ::= ipv6-hex ( ':' ipv6-hex )*
ipv6-hex         ::= ( DIGIT | [A-Fa-f] )*
port            ::= DIGIT+
```

**Examples:**
```ferron
example.com {
    root /var/www/example
}

*.example.com:80, example.org:443 {
    ssl enabled
}

http://api.example.com {
    proxy http://backend
}

[2001:db8::1]:8080 {
    root /ipv6-only
}
```

**Notes:**
- Host blocks are only allowed at the top level.
- The `*` wildcard matches any hostname or host label.
- IPv6 addresses must be enclosed in square brackets.

### 4.3 Global blocks

Global blocks apply configuration globally:

```ebnf
global-block    ::= block
```

**Example:**
```ferron
{
    default_timeout 30s
    log_format combined
}
```

**Notes:**
- Global blocks are only allowed at the top level.
- They contain statements that apply to all hosts unless overridden.

### 4.4 Snippet blocks

Snippet blocks define reusable configuration fragments:

```ebnf
snippet-block   ::= 'snippet' identifier block
```

**Example:**
```ferron
snippet ssl_config {
    ssl_certificate /etc/ssl/cert.pem
    ssl_key /etc/ssl/key.pem
}
```

Snippets can be referenced elsewhere in the configuration (implementation-dependent).

### 4.5 Match blocks

Match blocks define conditional logic based on request attributes:

```ebnf
match-block     ::= 'match' identifier matcher-block
matcher-block   ::= '{' matcher-expression* '}'
matcher-expression
                ::= operand operator operand
operator        ::= '==' | '!=' | '~' | '!~' | 'in'
operand         ::= identifier-path | string | number
```

**Examples:**
```ferron
match user_agent {
    request.header.user_agent == "Mozilla/5.0"
    request.header.x_forwarded_proto != "http"
    request.path ~ "^/api/"
    request.method !~ "(GET|HEAD)"
    request.method in "GET,HEAD"
}
```

**Operators:**
| Operator | Meaning | Example |
|----------|---------|---------|
| `==` | Equality | `Path == "/index.html"` |
| `!=` | Inequality | `Status != 404` |
| `~` | Regex match | `User-Agent ~ "Chrome.*"` |
| `!~` | Negated regex | `Host !~ "^test\."` |
| `in` | Membership | `Method in ["GET", "POST"]` |

## 5. Data types

### 5.1 Strings

Strings can be specified as:
- **Quoted strings** - enclosed in double quotes, support escape sequences (`\n`, `\r`, `\t`, `\\`)
- **Bare strings** - unquoted sequences of valid characters (alphanumeric, `_`, `-`, `.`, `:`, `/`, `*`, `+`)

**Escape sequences:**
| Escape | Character |
|--------|-----------|
| `\n` | newline |
| `\r` | carriage return |
| `\t` | tab |
| `\\` | backslash |
| `\"` | double quote |

### 5.2 Numbers

Numbers support integers and decimals:
```ebnf
number ::= '-'? DIGIT+ ( '.' DIGIT+ )?
```

**Examples:** `80`, `443`, `1.5`, `-10`

### 5.3 Booleans

Boolean literals are case-sensitive:
- `true` — enabled/positive value
- `false` — disabled/negative value

## 6. Interpolation

Interpolation allows referencing variables or configuration values:

```ebnf
interpolation ::= '{{' identifier-path '}}'
identifier-path ::= identifier ( '.' identifier )*
```

**Examples:**
```ferron
root "{{app.root}}"
port {{server.port}}
timeout {{config.defaults.timeout}}
```

## 7. Syntax examples

### Complete configuration example

***Note:** This configuration example may not reflect the directives the Ferron web server might use.*

```ferron
# Global defaults
{
    default_timeout 30s
    log_format combined
}

# Snippet definition
snippet ssl_config {
    ssl_certificate /etc/ssl/cert.pem
    ssl_key /etc/ssl/key.pem
    ssl_protocols TLSv1.2 TLSv1.3
}

# Host-specific configuration
example.com:443 {
    include ssl_config
    root /var/www/example
}

*.example.com {
    proxy http://backend.example.com
}

# Match-based routing
match api_rules {
    request.path ~ "^/api/"
    request.method in "GET,POST,PUT,DELETE"
}

api.example.com {
    match api_rules
    root /var/www/api
}
```

## 8. Error handling

### 8.1 Parse errors

The reference parser reports errors with:
- **Message** - description of the error
- **Span** - line and column position where the error occurred

### 8.2 Validation rules

- IPv4 octets must be in range 0–255 (validated by parser)
- Host patterns require proper formatting
- Match expressions require valid operands and operators

## 9. Implementation notes

### 9.1 Lexer behavior

- Bare strings are only allowed after certain token types (identifiers, numbers, operators) to avoid ambiguity.
- Comments are skipped between statements but not inside `match` blocks.
- The lexer is case-sensitive for keywords (`match`, `snippet`) and boolean values.

### 9.2 Parser behavior

- Host patterns can be comma-separated in host blocks.
- Interpolation syntax uses double braces `{{ }}`.
- Match expressions are evaluated sequentially within a match block.

## 10. Backward compatibility

This specification defines version 1.0 of the Ferron configuration format. Future versions may extend the grammar with additional features while maintaining backward compatibility where possible.
