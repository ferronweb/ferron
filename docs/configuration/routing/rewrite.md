---
title: "Configuration: URL rewriting"
description: "The `rewrite` directive for transforming request URLs with regular expression patterns."
---

`rewrite` transforms the request URL with a regular expression before proxying or static file serving. Later stages use the rewritten URL.

> [!important]
> A rewrite does not trigger a new round of `location` matching. Ferron selects the `location` block once, on the original URL, and strips the matched prefix before rewrite rules run. Rules therefore see the location-stripped path. Use `match` blocks with `if` or `if_not` for regex routing. See [Request pipeline order](/docs/configuration/fundamentals/request-pipeline).

> [!info]
> For `url_sanitize` interaction, see [HTTP protocol settings](/docs/configuration/server/host#http-protocol-settings). For static file serving, see [Static file serving](/docs/configuration/content/static-files).

## `rewrite`

`rewrite <regex: string> <replacement: string>` matches a regular expression and replaces the URL. Capture groups are available in the replacement as `$1`, `$2`, and so on. Default: none.

| Option                 | Arguments  | Description                                                                                                           | Default                    |
| ---------------------- | ---------- | --------------------------------------------------------------------------------------------------------------------- | -------------------------- |
| `last`                 | `<bool>`   | Stop processing further rewrite rules after this rule matches.                                                        | `false`                    |
| `directory`            | `<bool>`   | Apply this rule when the URL corresponds to a directory.                                                              | `true`                     |
| `file`                 | `<bool>`   | Apply this rule when the URL corresponds to a file.                                                                   | `true`                     |
| `allow_double_slashes` | `<bool>`   | Preserve double slashes (`//`) in the URL instead of collapsing them.                                                 | `false`                    |
| `name`                 | `<string>` | Rule identifier for rewrite observability output. Must be a plain string without interpolation.                       | derived from rule position |

```ferron
example.com {
    rewrite r"^/old-path/(.*)" "/new-path/$1"
}
```

Ferron rewrites `/old-path/anything` to `/new-path/anything` internally. The client sees no redirect.

> [!tip]
> If routing behaves unexpectedly, check rule order. A rule with `last` stops further processing.

### Stop processing with `last`

```ferron
example.com {
    rewrite r"^/api/v1/(.*)" "/api/v2/$1" {
        last
    }
    rewrite r"^/api/v2/(.*)" "/api/v3/$1"
}
```

Ferron rewrites `/api/v1/users` to `/api/v2/users` and stops. The second rule never sees the `/api/v2/` path.

### Chain rules

Without `last`, the next rule can rewrite the result again:

```ferron
example.com {
    rewrite r"^/legacy/(.*)" "/modern/$1" {
        name "legacy-to-modern"
    }
    rewrite r"^/modern/(.*)" "/current/$1" {
        name "modern-to-current"
    }
}
```

Ferron rewrites `/legacy/foo` to `/modern/foo`, then the second rule rewrites it to `/current/foo`. Give rules `name` values when several share a host, so observability output distinguishes them.

### Guard file and directory requests

`file false` and `directory false` skip the rule when the URL maps to a real file or directory under `root`. Use both guards for fallback patterns such as single-page apps and front controllers. Without the guards, a broad pattern also rewrites static asset requests, which breaks scripts, styles, and images.

```ferron
example.com {
    root /var/www

    rewrite r"^/static/(.*)" "/assets/$1" {
        file
        directory false
    }
}
```

> [!tip]
> Start every catch-all fallback rule with `file false` and `directory false`. Add `last` to stop rule processing after the fallback matches. Enable `rewrite_log true` while you test the guards.

## `rewrite_log`

`rewrite_log <bool>` logs each URL rewrite operation to the error log. Default: `false`.

```ferron
example.com {
    rewrite_log
}
```

Example log output:

```text
URL rewritten from "/old-path/users" to "/new-path/users"
```

## Regex syntax

The engine is [`regex`](https://crates.io/crates/regex), which runs regular expressions in linear time without backtracking. This mitigates ReDoS and catastrophic backtracking attacks.

These constructs are not available:

- Lookahead: `(?= ...)`, `(?! ...)`
- Lookbehind: `(?<= ...)`, `(?<! ...)`
- Backreferences to capture groups: `\1`

To negate a condition, use a negated variant such as `if_not` instead of `if`, or `!~` in matchers instead of `~`.

Matching is case-insensitive on Windows and case-sensitive on other platforms.

## Observability

Rule positions are 1-based. The first `rewrite` directive in a block is rule 1. Give rules `name` values when you need readable identifiers instead of positions.

### Metrics

| Metric                            | Type    | Attributes                                                           | Description                                                                                    |
| --------------------------------- | ------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `ferron.rewrite.rewrites_applied` | Counter | `ferron.rewrite.rule_index`, `ferron.rewrite.rule_name` (when named) | Rewrite rule firings. One increment per matched rule, so chained rewrites count once per step. |
| `ferron.rewrite.invalid`          | Counter | `ferron.rewrite.rule_index`, `ferron.rewrite.rule_name` (when named) | Rewrite rules that produced an invalid path, which results in a 400 response.                  |

### Logs

When `rewrite_log` is on, Ferron logs each rewrite operation to the error log at `DEBUG` level.

### Structured logs

| Description (summary) | Level | Attributes                                                                                                                                                                                                                                                                                                                                                                     |
| --------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| URL rewritten         | INFO  | `ferron.rewrite.from` (string): path and query string before the step. `ferron.rewrite.to` (string): path and query string after the step. `ferron.rewrite.rule_index` (int): the fired rule. `ferron.rewrite.rule_name` (string): present when the rule is named. Ferron emits one line per fired rule, so chained rewrites produce one line per step.                       |

### Access log fields

The rewrite module adds these fields to the HTTP access log line:

| Field                          | Type   | Description                                          |
| ------------------------------ | ------ | ---------------------------------------------------- |
| `ferron.rewrite.applied`       | bool   | Whether Ferron applied a URL rewrite to the request. |
| `ferron.rewrite.matched_rules` | int    | Number of rewrite rules that fired for the request.  |
| `ferron.rewrite.rule_index`    | int    | 1-based position of the first fired rule.            |
| `ferron.rewrite.rule_name`     | string | Name of the first fired rule, when it sets `name`.   |

### Trace spans

The rewrite stage sets these attributes on its `ferron.stage.rewrite` span:

| Attribute                           | Type   | Description                                                      |
| ----------------------------------- | ------ | ---------------------------------------------------------------- |
| `ferron.rewrite.applied`            | bool   | Whether Ferron applied a rewrite rule to the request.            |
| `ferron.rewrite.pattern_count`      | int    | Number of rewrite rules evaluated, not the number that matched.  |
| `ferron.rewrite.matched_rule_count` | int    | Number of rewrite rules that fired for the request.              |
| `ferron.rewrite.rule_index`         | int    | 1-based position of the first fired rule.                        |
| `ferron.rewrite.rule_name`          | string | Name of the first fired rule, when it sets `name`.               |
