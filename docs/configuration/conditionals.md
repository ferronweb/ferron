---
title: "Configuration: conditionals"
description: "How to define and apply conditional configuration with condition, if, if_not, subconditions, and Rego."
---

This page documents Ferron conditionals overall: how to define reusable conditions, apply conditional blocks, and use subconditions (including Rego).

## Conditionals overview

Ferron 2.0.0 and newer support conditional configuration. This lets you apply different directives based on request properties, headers, IP addresses, and custom policy logic.

A conditional flow has three parts:

- `condition "<name>" { ... }` defines a named condition.
- Subconditions inside `condition` are checked together.
- `if "<name>" { ... }` and `if_not "<name>" { ... }` apply directives when the condition passes or fails.

## `condition`, `if`, and `if_not`

Use `condition` to define reusable checks, then attach behavior with `if` and `if_not`.

```kdl
example.com {
  condition "IS_API" {
    is_regex "{path}" "^/api(/|$)"
  }

  if "IS_API" {
    proxy "http://127.0.0.1:3000"
  }

  if_not "IS_API" {
    root "/var/www/html"
  }
}
```

**Rules:**

- A condition passes only if all subconditions in that `condition` block pass.
- Conditions can be nested by placing `if`/`if_not` blocks inside other conditional blocks.
- In Ferron 2.1.0 and newer, snippets containing subconditions can be reused in condition blocks with `use`.

For broader block structure and ordering details, see [Configuration: fundamentals](/docs/configuration/fundamentals).

## Subconditions reference

Supported subconditions:

- `is_remote_ip <remote_ip: string> [<remote_ip: string> ...]`
  - Checks whether the request is coming from a specific remote IP address (or one of multiple addresses).
- `is_forwarded_for <remote_ip: string> [<remote_ip: string> ...]`
  - Checks whether the request (respecting the `X-Forwarded-For` header) is coming from a specific forwarded IP address (or one of multiple addresses).
- `is_not_remote_ip <remote_ip: string> [<remote_ip: string> ...]`
  - Checks whether the request is not coming from a specific remote IP address (or list of addresses).
- `is_not_forwarded_for <remote_ip: string> [<remote_ip: string> ...]`
  - Checks whether the request (respecting the `X-Forwarded-For` header) is not coming from a specific forwarded IP address (or list of addresses).
- `is_equal <left_side: string> <right_side: string>`
  - Checks whether the left side equals the right side.
- `is_not_equal <left_side: string> <right_side: string>`
  - Checks whether the left side does not equal the right side.
- `is_regex <value: string> <regex: string> [case_insensitive=<case_insensitive: bool>]`
  - Checks whether the value matches the regular expression. `case_insensitive` controls case sensitivity (`#false` by default).
- `is_not_regex <value: string> <regex: string> [case_insensitive=<case_insensitive: bool>]`
  - Checks whether the value does not match the regular expression. `case_insensitive` controls case sensitivity (`#false` by default).
- `is_rego <rego_policy: string>`
  - Evaluates an embedded Rego policy.
- `set_constant <key: string> <value: string>` (Ferron 2.1.0 or newer)
  - Sets a constant value.
- `is_language <language: string>` (Ferron 2.1.0 or newer)
  - Checks whether the language is preferred in the `Accept-Language` header. This uses the `LANGUAGES` constant (comma-separated language codes such as `en-US` or `fr-FR`).

Placeholders can be used in subconditions where applicable (for example `{path}` and `{client_ip}`). See [Configuration: placeholders](/docs/configuration/placeholders).

## Rego in conditionals

**Note: Ferron previously supported Rego-based subconditions for advanced access control. This feature is now deprecated and will be removed in a future release.**

**For most use cases, standard conditionals are sufficient and recommended. Advanced policy logic will instead be provided by a future API gateway from Ferron, which will offer full Rego support in a dedicated, lean, and extensible environment**

**Existing configurations using Rego subconditions will continue to work for now, but it is advised to migrate to standard conditional or plan for the future API gateway when it comes available.**

Ferron 2.0.0 and newer support advanced conditional checks with embedded Rego policy.

When writing Rego policies for Ferron conditionals:

- Set the package to `ferron` (`package ferron`).
- Return the decision in `pass`.

Inputs for Rego-based subconditions (`input`) are as follows:

- `input.method` (string) - the HTTP method of the request (`GET`, `POST`, etc.)
- `input.uri` (string) - the URI of the request (for example, `/index.php?page=1`)
- `input.headers` (array<string, array<string>>) - the headers of the request. The header names are in lower-case.
- `input.socket_data.client_ip` (string) - the client's IP address.
- `input.socket_data.client_port` (number) - the client's port.
- `input.socket_data.server_ip` (string) - the server's IP address.
- `input.socket_data.server_port` (number) - the server's port.
- `input.socket_data.encrypted` (boolean) - whether the connection is encrypted.
- `input.constants` (array<string, string>; Ferron 2.1.0 or newer) - the constants set by `set_constant` subconditions.

You can read more about Rego in [Open Policy Agent documentation](https://www.openpolicyagent.org/docs/policy-language).

**Configuration example with Rego (`curl` user-agent deny):**

```kdl
// Replace "example.com" with your domain name.
example.com {
  condition "DENY_CURL" {
    is_rego """
      package ferron

      default pass := false

      pass := true if {
        input.headers["user-agent"][0] == "curl"
      }

      pass := true if {
        startswith(input.headers["user-agent"][0], "curl/")
      }
      """
  }

  if "DENY_CURL" {
    status 403
  }

  // Serve static files
  root "/var/www/html"
}
```
