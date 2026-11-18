---
title: "Configuration: canary deployments"
description: "Weighted, sticky variant selection for canary rollouts and A/B testing of static content."
---

`canary` assigns each request a variant from a weighted list. The assignment stays stable for a given client. Use it to roll out content gradually or run A/B tests without a reverse proxy.

## `canary`

`canary <name: string> { ... }` assigns a variant from the sticky key and the configured variant weights. Ferron evaluates blocks in declaration order and uses the first block that matches the current host. Default: none.

| Sub-directive | Arguments                                                    | Description                                                                                                                                                      | Default |
| ------------- | ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `affinity`    | `ip`, `cookie <name>`, `header <name>`, or `hash <variable>` | Sticky key source. With `cookie` or `header`, Ferron uses the value of the named cookie or header. With `hash`, Ferron uses the value of the named variable.      | `ip`    |
| `set_cookie`  | `[bool]`                                                     | Set the affinity cookie when the request has none. Valid only with `cookie` affinity.                                                                            | `false` |
| `variant`     | `<value: string> <weight: number>`                           | Declares one variant with its weight. Repeat the directive for more variants. Weights must be at least 1.                                                       | none    |
| `cookie`      | `{ ... }`                                                    | Configures affinity cookie attributes for `set_cookie`. See [Cookie block](#cookie-block).                                                                       | none    |

```ferron
example.com {
    canary rollout {
        variant stable 90
        variant next 10
    }
}
```

Ferron assigns about 90% of requests to `stable` and 10% to `next`. Each client IP stays on the same variant while the weights do not change.

### Cookie block

The `cookie` block sets the attributes of the affinity cookie that Ferron writes when `set_cookie` is enabled. Without this block, Ferron writes a persistent cookie that survives browser restarts, with `HttpOnly` and `SameSite=Lax`.

> [!note]
> The `cookie` block applies only with `set_cookie` and cookie affinity. It does not change how Ferron reads an existing cookie.

| Sub-directive | Arguments         | Description                                                                                            | Default             |
| ------------- | ----------------- | ------------------------------------------------------------------------------------------------------ | ------------------- |
| `ttl`         | `<duration>`      | How long the cookie lasts. Ferron writes it as `Max-Age`. Omit it to keep the browser-session default. | `7d` (persistent)   |
| `path`        | `<value: string>` | Cookie path.                                                                                           | `/`                 |
| `domain`      | `<value: string>` | Cookie domain. Omit it to scope the cookie to the current host.                                        | none (current host) |
| `secure`      | `[bool]`          | Set the `Secure` flag so the cookie is sent only over HTTPS.                                           | `false`             |
| `httponly`    | `[bool]`          | Set the `HttpOnly` flag so client scripts cannot read the cookie.                                      | `true`              |
| `samesite`    | `<policy>`        | `SameSite` policy: `strict`, `lax`, or `none`.                                                         | `lax`               |

```ferron
example.com {
    canary rollout {
        affinity cookie ab_variant
        set_cookie
        variant stable 90
        variant next 10
        cookie {
            ttl "30d"
            domain example.com
            secure
            samesite lax
        }
    }
}
```

Ferron writes the `ab_variant` cookie for 30 days, scoped to `example.com`, and sends it only over HTTPS.

## Pipeline position

The canary stage runs after client IP resolution and before the `set_var` and `map` stages. Variants are available for variable interpolation, `map` evaluation, and all downstream stages.

## Variables

Ferron sets these variables during the canary stage:

| Variable         | Description                                         |
| ---------------- | --------------------------------------------------- |
| `canary.variant` | The selected variant name.                          |
| `canary.weight`  | The weight of the selected variant.                 |
| `canary.key`     | The sticky key value Ferron used for the selection. |

### Serve content per variant

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

Ferron interpolates `canary.variant` per request, so each variant serves its own document root without a reload.

### Branch with `set_var`

```ferron
example.com {
    canary rollout {
        variant stable 90
        variant next 10
    }
    set_var canary.variant r"^next$" is_next {
        value "true"
    }
    #...
}
```

## Client affinity

By default, Ferron hashes the client IP address. A client keeps its variant while the weights do not change. Change the key with `affinity`.

### Cookie

```ferron
example.com {
    canary ab_test {
        affinity cookie ab_variant
        variant control 50
        variant experiment 50
    }
}
```

Ferron hashes the `ab_variant` cookie value. If the cookie is missing, Ferron falls back to the client IP. Use this for A/B tests where the client application sets the cookie.

> [!note]
> Ferron does not set the cookie itself. The client application must set it before the first request. To make Ferron set the cookie, add `set_cookie`.

### Let Ferron set the cookie

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

When the request has no `ab_variant` cookie, Ferron generates a random sticky key, assigns the variant from the ring, and writes the cookie to the response. By default the cookie is persistent (`ab_variant=<key>; Path=/; Max-Age=604800; HttpOnly; SameSite=Lax`), so the assignment survives browser restarts and IP changes. Use the [Cookie block](#cookie-block) to change the lifetime and other attributes. Use `set_cookie false` to disable. `set_cookie` works only with cookie affinity.

### Header

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

### Variable hash

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
> `hash` affinity runs before the `set_var` and `map` stages, so the variable must be a built-in request variable such as `request.cookie.<name>`, `request.header.<name>`, or `request.uri.query.<param>`. Variables produced by `set_var` or `map` are not available yet.

## Promotion and rollback

The variant weights and the variant list can change on reload. Ferron keeps the assignment for clients whose keys map to an unchanged part of the hash ring. Clients near a variant boundary can move to another variant, which is expected with consistent hashing.

To promote a variant, increase its weight over several reloads. To roll back, decrease the weight or set `variant stable 100`.

For weighted load balancing across backend servers, see [HTTP reverse proxy](/docs/configuration/proxy/reverse-proxy).

## Observability

### Trace spans

The canary stage sets these attributes on its `ferron.stage.canary` span:

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

## See also

- [HTTP map](/docs/configuration/routing/map)
- [Conditionals and variables](/docs/configuration/fundamentals/conditionals)
- [Static file serving](/docs/configuration/content/static-files)
