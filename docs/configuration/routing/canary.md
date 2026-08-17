---
title: "Configuration: canary deployments"
description: "The `canary` directive for weighted, sticky variant selection for canary rollouts and A/B testing of static content."
---

This page documents the `canary` directive. It assigns each request a variant from a weighted list and keeps the assignment stable for a given client, so you can roll out new content gradually or run A/B tests without a reverse proxy.

## Directives

### `canary`

- `canary <name: string> { ... }` (`http-canary`)
  - Assigns a variant to each request based on a sticky key and the configured variant weights. Ferron evaluates the block in declaration order and selects the first block that matches the current host. Default: none

#### Block sub-directives

| Sub-directive | Arguments                                                    | Description                                                                                                                                                      | Default |
| ------------- | ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `affinity`    | `ip`, `cookie <name>`, `header <name>`, or `hash <variable>` | The sticky key source. With `cookie` or `header`, Ferron uses the value of the named cookie or header. With `hash`, Ferron uses the value of the named variable. | `ip`    |
| `set_cookie`  | `[bool]`                                                     | When `true`, Ferron sets the affinity cookie itself when the request has none. Valid only with `cookie` affinity.                                                | `false` |
| `variant`     | `<value: string> <weight: number>`                           | Declares one variant with its weight. Repeat the directive to declare more variants. Weights must be at least 1.                                                 | none    |

**Configuration example:**

```ferron
example.com {
    canary rollout {
        variant stable 90
        variant next 10
    }
}
```

Ferron assigns about 90% of requests to `stable` and 10% to `next`. Each client IP stays on the same variant across requests, as long as the weights do not change.

## Pipeline position

The `canary` stage runs after client IP resolution and before the `set_var` and `map` stages. This means variants are available for variable interpolation, `map` evaluation, and all downstream stages.

## Using the variant in configuration

Ferron sets the following variables during the canary stage:

| Variable         | Description                                         |
| ---------------- | --------------------------------------------------- |
| `canary.variant` | The selected variant name.                          |
| `canary.weight`  | The weight of the selected variant.                 |
| `canary.key`     | The sticky key value Ferron used for the selection. |

**Serving different content per variant:**

```ferron
example.com {
    canary rollout {
        variant stable 90
        variant next 10
    }
    root "/srv/www/{{canary.variant}}"
    index index.html
}
```

Ferron interpolates the `canary.variant` variable per request, so each variant serves its own document root without a reload.

**Branching with `set_var`:**

```ferron
example.com {
    canary rollout {
        variant stable 90
        variant next 10
    }
    set_var canary.variant "^next$" is_next {
        value "true"
    }
    #...
}
```

## Client affinity

By default, Ferron hashes the client IP address. A client keeps its variant as long as the weights do not change. Change this with the `affinity` sub-directive.

**Keeping each visitor on one variant with a cookie:**

```ferron
example.com {
    canary ab_test {
        affinity cookie ab_variant
        variant control 50
        variant experiment 50
    }
}
```

Ferron hashes the `ab_variant` cookie value. If the cookie is missing, Ferron falls back to the client IP. This is useful for A/B tests where the client application sets the cookie.

> [!note]
> Ferron does not set the cookie itself. The client application sets it before the first request. To make Ferron set the cookie, add `set_cookie` (see below).

**Making Ferron set the cookie:**

```ferron
example.com {
    canary rollout {
        affinity cookie ab_variant
        set_cookie
        variant stable 90
        variant next 10
    }
}
```

When the request has no `ab_variant` cookie, Ferron generates a random sticky key, assigns the variant from the ring, and writes the cookie to the response (`ab_variant=<key>; Path=/`). The client sends the cookie back on later requests, so the assignment survives IP changes. Use `set_cookie false` to disable, and note that `set_cookie` works only with `cookie` affinity.

**Using a request header:**

```ferron
example.com {
    canary rollout {
        affinity header x_canary_group
        variant stable 80
        variant eastus 20
    }
}
```

Ferron hashes the value of the `X-Canary-Group` header. Header names are case-insensitive. If the header is missing, Ferron falls back to the client IP.

**Hashing a variable:**

```ferron
example.com {
    canary rollout {
        affinity hash request.cookie.user_id
        variant stable 80
        variant premium 20
    }
}
```

> [!note]
> The `hash` affinity runs before the `set_var` and `map` stages, so the variable must be a built-in request variable such as `request.cookie.<name>`, `request.header.<name>`, or `request.uri.query.<param>`. Variables produced by `set_var` or `map` are not available yet at this point.

## Promotion and rollback

The variant weights and the variant list can change on reload. Ferron keeps the assignment for clients whose keys map to a part of the hash ring that did not change. Clients near a variant boundary may move to another variant, which is expected with consistent hashing.

To promote a variant, increase its weight over several reloads. To roll back, decrease the weight or change it to `variant stable 100`.

For weighted load balancing across multiple backend servers, see [HTTP reverse proxy](/docs/v3/configuration/proxy/reverse-proxy).

## Observability

### Trace spans

The canary stage sets the following attributes on its `ferron.stage.canary` span:

| Attribute                  | Type   | Description                                                                                                                |
| -------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------- |
| `ferron.canary.variant`    | string | The selected variant name.                                                                                                 |
| `ferron.canary.name`       | string | The canary block name.                                                                                                     |
| `ferron.canary.key_source` | string | Where the key came from: `ip`, `cookie`, `header`, `hash`, or `generated` (a random key persisted in the affinity cookie). |
| `ferron.canary.weight`     | int    | The weight of the selected variant.                                                                                        |

### Metrics

| Metric                   | Type    | Description                                                                                                                                |
| ------------------------ | ------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `ferron.canary.requests` | Counter | Total requests processed by canary blocks. The attributes `ferron.canary.name` and `ferron.canary.variant` distinguish series per variant. |

### Access log fields

Ferron writes the selected variant to the custom access log field `ferron.canary.variant`. See [Logging](../observability/logging.md) for how to enable custom fields.

> [!info]
> For variable mapping and conditional logic, see [HTTP map](/docs/v3/configuration/routing/map) and [Conditionals and variables](/docs/v3/configuration/fundamentals/conditionals). For serving each variant from its own directory, see [Static file serving](/docs/v3/configuration/content/static-files).
