---
title: "Configuration: subconditions"
description: "Conditional config subconditions, including regex and Rego-based policy checks."
---

This page documents KDL subconditions that power conditional configuration, from basic comparisons to Rego policy checks.

## Subconditions

Ferron 2.0.0 and newer supports conditional configuration based on conditions. This allows you to configure different settings based on the request method, path, or other conditions.

Below is the list of supported subconditions:

- `is_remote_ip <remote_ip: string> [<remote_ip: string> ...]`
  - This subcondition checks if the request is coming from a specific remote IP address or a list of IP addresses.
- `is_forwarded_for <remote_ip: string> [<remote_ip: string> ...]`
  - This subcondition checks if the request (with respect for `X-Forwarded-For` header) is coming from a specific forwarded IP address or a list of IP addresses.
- `is_not_remote_ip <remote_ip: string> [<remote_ip: string> ...]`
  - This subcondition checks if the request is not coming from a specific remote IP address or a list of IP addresses.
- `is_not_forwarded_for <remote_ip: string> [<remote_ip: string> ...]`
  - This subcondition checks if the request (with respect for `X-Forwarded-For` header) is not coming from a specific forwarded IP address or a list of IP addresses.
- `is_equal <left_side: string> <right_side: string>`
  - This subcondition checks if the left side is equal to the right side.
- `is_not_equal <left_side: string> <right_side: string>`
  - This subcondition checks if the left side is not equal to the right side.
- `is_regex <value: string> <regex: string> [case_insensitive=<case_insensitive: bool>]`
  - This subcondition checks if the value matches the regular expression. The `case_insensitive` prop specifies whether the regex should be case insensitive (`#false` by default).
- `is_not_regex <value: string> <regex: string> [case_insensitive=<case_insensitive: bool>]`
  - This subcondition checks if the value does not match the regular expression. The `case_insensitive` prop specifies whether the regex should be case insensitive (`#false` by default).
- `is_rego <rego_policy: string>`
  - This subcondition evaluates a Rego policy.
- `set_constant <key: string> <value: string>` (Ferron 2.1.0 or newer)
  - This subcondition sets a constant value.
- `is_language <language: string>` (Ferron 2.1.0 or newer)
  - This subcondition checks if the language is the preferred language specified in the `Accept-Language` header. This subcondition uses `LANGUAGES` constant, which is a comma-separated list of preferred language codes (such as `en-US` or `fr-FR`).

## Rego subconditions

Ferron 2.0.0 and newer supports more advanced subconditions with Rego policies embedded in Ferron configuration.

When writing Rego policies for Ferron subconditions, you need to set the package name to `ferron` (`package ferron` line). Ferron checks the `pass` result of the policy.

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

**Configuration example utilizing Rego subconditions (denying `curl` requests):**

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
