---
title: "Configuration: request pipeline order"
description: "The order in which Ferron resolves configuration and runs request pipeline stages."
---

This page states the order in which Ferron resolves configuration and runs request pipeline stages. Use it when rewrite rules, `location` blocks, maps, or proxy and file handling interact in unexpected ways.

> [!info]
> For `location` and conditional matching, see [Routing and URL processing](/docs/configuration/routing/url-processing). For rewrite reference, see [URL rewriting](/docs/configuration/routing/rewrite). For maps, see [HTTP map](/docs/configuration/routing/map).

## Configuration resolution runs first

Ferron resolves configuration once per request, before any pipeline stage runs. Resolution uses the original sanitized request URL.

1. Ferron loads global defaults from the bare `{ ... }` block.
2. Ferron selects matching host blocks by listener IP and hostname.
3. Ferron merges matching `location` blocks. Matching is prefix only. The longest match wins.
4. Ferron merges matching `if` and `if_not` blocks.

After resolution, Ferron strips the matched `location` prefix from the request path. Pipeline stages then see the stripped path.

> [!important]
> Rewrites do not trigger a new round of `location` matching. Ferron selects the `location` block once, on the original URL. A rewrite changes the URL for later stages such as proxying or file serving. It does not move the request to a different `location` block.

## Pipeline stage order

Stages run in a fixed partial order. The list below shows the typical sequence for an HTTP request. Some stages skip the request when their directives are absent.

1. ACME HTTP-01 challenge answer. It handles `/.well-known/acme-challenge/*` before other work.
2. Client IP resolution (`client_ip_from_header`).
3. HTTP to HTTPS redirect (`https_redirect`).
4. Canary assignment. It sets `canary.*` variables.
5. Variable setting (`set_var`).
6. Map evaluation (`map`). Maps can use variables from steps 4 and 5.
7. Response control (`status`, `abort`, `allow`, `block`).
8. URL rewriting (`rewrite`). Rules can use variables from steps 4 through 6. Rules see the location stripped path.
9. Request body buffering (`buffer`).
10. Abuse protection, rate limiting, and authentication (`basic_auth`, forwarded auth).
11. Cache lookup.
12. Content stages. Only one responds. Ferron tries forward proxy, reverse proxy, CGI, FastCGI, SCGI, static files, directory listings, and error pages in constraint order.
13. Post response work. It includes dynamic compression, header changes, response body replacement, and custom log fields (`log_field`).

> [!note]
> Steps 7 and 8 have no strict order guarantee between each other. Both run after redirects and before proxying and file serving. Design rules so the result does not depend on which of the two runs first.

## Consequences for common patterns

- Put regex routing in `match` blocks with `if` or `if_not`. Do not expect regex in `location` blocks. `location` supports prefixes only.
- Guard broad rewrite patterns with `file false` and `directory false`. A catch-all pattern such as `^/(.*)` also matches static asset paths. The guards keep real files and directories on the file path.
- Set variables and maps before rewrites use them. Steps 4 through 6 run before step 8, so `rewrite` patterns and replacements can read mapped variables.
- Debug rewrites with `rewrite_log true`. The error log shows each rewrite from the original to the rewritten URL.

## See also

- [Routing and URL processing](/docs/configuration/routing/url-processing)
- [URL rewriting](/docs/configuration/routing/rewrite)
- [HTTP map](/docs/configuration/routing/map)
- [Variable setting](/docs/configuration/routing/variables)
- [Syntax and file structure](/docs/configuration/fundamentals/syntax)
