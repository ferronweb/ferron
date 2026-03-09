---
title: "Configuration: fundamentals"
description: "KDL configuration basics: blocks, include patterns, directive scopes."
---

Ferron 2.0.0 and newer can be configured in a [KDL-format](https://kdl.dev/) configuration file (often named `ferron.kdl`).

## KDL syntax quick reference

Ferron configuration uses KDL nodes as directives:

- The node name is the directive name (for example, `root` or `location`).
- Positional arguments are written after the directive name.
- Named properties use `key=value`.
- Booleans and null use KDL literals: `#true`, `#false`, `#null`.
- Use double quotes for strings that contain special characters (such as `/`, `.`, `:`, spaces, or commas).
- Line comments start with `//`.

```kdl
example.com {
  root "/var/www/example.com"
  location "/assets" remove_base=#true {
    file_cache_control "public, max-age=31536000"
  }

  limit rate=100 burst=200
  protocol_proxy #false
  timeout #null
}
```

## Configuration blocks

At the top level of the server configuration, you define blocks that represent virtual hosts and global scopes. Below are examples of such blocks:

```kdl
globals {
  // Global configuration that doesn't imply any virtual host
}

* {
  // Global configuration
}

*:80 {
  // Configuration for port 80
}

example.com {
  // Configuration for "example.com" virtual host
}

// Hostnames starting with a number need to be quoted, due to constraints of KDL syntax.
"1.example.com" {
  // Configuration for "1.example.com" virtual host
}

"192.168.1.1" {
  // Configuration for "192.168.1.1" IP virtual host
}

example.com:8080 {
  // Configuration for "example.com" virtual host with port 8080
}

"192.168.1.1:8080" {
  // Configuration for "192.168.1.1" IP virtual host with port 8080
}

api.example.com {
  // Below is the location configuration for paths beginning with "/v1/". If there was "remove_base=#true", the request URL for the location would be rewritten to remove the base URL
  location "/v1" remove_base=#false {
    // ...

    // Below is the error handler configuration for any status code.
    error_config {
      // ...
    }
  }

  // The location and conditionals' configuration order is automatically determined based on the location and conditionals' depth
  location "/" {
    // ...
  }

  // Below is the error handler configuration for 404 Not Found status code. If "404" wasn't included, it would be for all errors.
  error_config 404 {
    // ...
  }
}

example.com,example.org {
  // Configuration for example.com and example.org
  // The virtual host identifiers (like example.com or "192.168.1.1") are comma-separated, but adding spaces will not be interpreted,
  // For example "example.com, example.org" will not work for "example.org", but "example.com,example.org" will work.
}

with-conditions.example.com {
  condition "SOME_CONDITION" {
    // Here are defined subconditions in the condition. The condition will pass if all subconditions will also pass
  }

  if "SOME_CONDITION" {
    // Conditional configuration
    // Conditions can be nested
  }

  if_not "SOME_CONDITION" {
    // Configuration, in case of condition not being met
  }
}

snippet "EXAMPLE" {
  // Example snippet configuration
  // Snippets containing subconditions can also be used in conditions in Ferron 2.1.0 or newer
}

with-snippet.example.com {
  // Import from snippet
  use "EXAMPLE"
}

with-snippet.example.org {
  // Snippets can be reusable
  use "EXAMPLE"
}

inheritance.example.com {
  // The "proxy" directive is used as an example for demonstrating inheritance.
  proxy "http://10.0.0.2:3000"
  proxy "http://10.0.0.3:3000"

  // Here, these directives take effect:
  //   proxy "http://10.0.0.2:3000"
  //   proxy "http://10.0.0.3:3000"

  location "/somelocation" {
    // Here, `some_directive` directives are inherited from the parent block.
    // These directives take effect:
    //   proxy "http://10.0.0.2:3000"
    //   proxy "http://10.0.0.3:3000"
  }

  location "/anotherlocation" {
    // The directives from the parent block are not inherited if there are other directives with the same name in the block.
    // Here, these directives take effect:
    //   proxy "http://10.0.0.4:3000"
    proxy "http://10.0.0.4:3000"
  }
}
```

Also, it's possible to include other configuration files using an `include <included_configuration_path: string>` directive, like this:

```kdl
include "/etc/ferron.d/**/*.kdl"
```

## Organizing configuration with `include`

`include` is useful when your configuration grows beyond one file. A common pattern is to keep reusable or shared defaults separate from per-site files:

```kdl
include "/etc/ferron.d/core/*.kdl"
include "/etc/ferron.d/sites/*.kdl"
```

In practice, this makes it easier to:

- keep global defaults in one place
- split virtual hosts into smaller files
- reuse snippets and condition definitions across files (supported on Ferron UNRELEASED and newer)

## Reuse and branching fundamentals

Reusable blocks and conditional branches are central to Ferron configuration:

- `snippet "<name>" { ... }` defines reusable directives.
- `use "<name>"` imports a snippet where needed.
- `condition "<name>" { ... }` defines reusable checks.
- `if "<name>" { ... }` and `if_not "<name>" { ... }` apply directives based on the condition result.

Example pattern:

```kdl
snippet "SECURITY_HEADERS" {
  header "X-Frame-Options" "DENY"
  header "X-Content-Type-Options" "nosniff"
}

example.com {
  condition "IS_API" {
    is_regex "{path}" "^/api(/|$)"
  }

  if "IS_API" {
    use "SECURITY_HEADERS"
    proxy "http://127.0.0.1:3000"
  }
}
```

For a full list of subconditions, see [Configuration: conditionals](/docs/configuration/conditionals). For larger real-world patterns, see [Configuration: configuration examples](/docs/configuration/examples).

## Inheritance and override behavior

Ferron applies inheritance by block context:

- Location blocks inherit parent directives unless the child block defines directives with the same name.
- When a child block defines a directive name, the child's entries for that directive take precedence in that block.
- For conditional branches, it is often clearer to explicitly `use` shared snippets inside each branch.

## Directive scopes

Ferron configuration directives have different scopes, which determine where they can be used within the configuration file. Some directives can only be used in the global scope, while others can be used in both global and virtual host scopes, or even in more specific contexts like location blocks.

In Ferron, configuration directives can be categorized into the following scopes:

- **Global-only** - can only be used in the global configuration scope
- **Global and virtual host** - can be used in both global and virtual host scopes
- **General directives** - can be used in various scopes including virtual hosts and location blocks

## Common mistakes

### 1. Not quoting hosts that require quotes

Some virtual host identifiers must be quoted to be valid KDL nodes (for example, hostnames that start with a number and IP-based identifiers with ports).

```kdl
// Correct
"1.example.com" { }
"192.168.1.1:8080" { }
```

### 2. Adding spaces in comma-separated virtual host blocks

Multiple host identifiers in one block are comma-separated without spaces.

```kdl
// Correct
example.com,example.org { }

// Incorrect (second host won't match as expected)
example.com, example.org { }
```

### 3. Expecting parent snippet usage to carry into every conditional branch

In complex configurations, explicitly reusing shared snippets inside `if`/`if_not` branches avoids surprises and keeps behavior clear.

```kdl
snippet "SECURITY_HEADERS" {
  header "X-Frame-Options" "DENY"
}

example.com {
  condition "IS_API" {
    is_regex "{path}" "^/api(/|$)"
  }

  if "IS_API" {
    use "SECURITY_HEADERS"
    proxy "http://127.0.0.1:3000"
  }

  if_not "IS_API" {
    use "SECURITY_HEADERS"
    root "/var/www/html"
  }
}
```
