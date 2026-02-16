---
title: "Configuration: fundamentals"
description: "KDL configuration basics: blocks, include patterns, directive scopes."
---

Ferron 2.0.0 and newer can be configured in a [KDL-format](https://kdl.dev/) configuration file (often named `ferron.kdl`).

## Configuration blocks

At the top level of the server configuration, the confguration blocks representing specific virtual host are specified. Below are the examples of such configuration blocks:

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

## Directive scopes

Ferron configuration directives have different scopes, which determine where they can be used within the configuration file. Some directives can only be used in the global scope, while others can be used in both global and virtual host scopes, or even in more specific contexts like location blocks.

### Scopes

- **Global-only** - can only be used in the global configuration scope
- **Global and virtual host** - can be used in both global and virtual host scopes
- **General directives** - can be used in various scopes including virtual hosts and location blocks
