# HTTP Host Directives

These directives are consumed from HTTP host blocks such as:

```ferron
example.com {
}

http example.com:8080 {
}
```

## Categories

- Protocol behavior: `http`
- TLS: `tls`
- Observability: `observability`

## `http`

Syntax:

```ferron
example.com {
    http {
        protocols h1 h2
        h1_enable_early_hints false
        h2_initial_window_size 65535
        h2_max_frame_size 32768
        h2_max_concurrent_streams 128
        h2_max_header_list_size 16384
        h2_enable_connect_protocol false
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `protocols` | `<string>...` | Enabled HTTP protocols. Currently supported values are `h1` and `h2`. | `h1 h2` |
| `h1_enable_early_hints` | `<bool>` | Enables HTTP/1.1 early hints support. | `false` |
| `h2_initial_window_size` | `<number>` | HTTP/2 initial flow-control window size. Must resolve to a non-negative integer at runtime. | unset |
| `h2_max_frame_size` | `<number>` | HTTP/2 maximum frame size. Must resolve to a non-negative integer at runtime. | unset |
| `h2_max_concurrent_streams` | `<number>` | HTTP/2 maximum concurrent streams. Must resolve to a non-negative integer at runtime. | unset |
| `h2_max_header_list_size` | `<number>` | HTTP/2 maximum header list size. Must resolve to a non-negative integer at runtime. | unset |
| `h2_enable_connect_protocol` | `<bool>` | Enables the HTTP/2 extended CONNECT protocol setting. | `false` |

Notes:

- `protocols` must leave at least one supported protocol enabled.
- `h3` is currently rejected.

## `tls`

Preferred syntax:

```ferron
example.com {
    tls true {
        provider manual
        cert "{{env.TLS_CERT}}"
        key "{{env.TLS_KEY}}"
    }
}
```

Accepted by the current validator:

- `tls <bool> { ... }`
- `tls <string> <string>`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `provider` | `<string>` | TLS provider name. Required when TLS is enabled through the block form. | none |

Current runtime behavior:

- If `tls` is absent, TLS is disabled for that host.
- If `tls false { ... }` is used, the block is ignored.
- The HTTP server currently reads the nested `provider` directive and then delegates the rest of the block to the selected TLS provider.

Bundled provider-specific options:

### `provider manual`

The bundled `manual` TLS provider reads:

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `cert` | `<string>` | Path to a PEM certificate file. Interpolation is supported. | none |
| `key` | `<string>` | Path to a PEM private key file. Interpolation is supported. | none |

## `observability`

Syntax:

```ferron
example.com {
    observability true {
        provider console
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `provider` | `<string>` | Observability provider name. Required when observability is enabled through the block form. | none |

Current runtime behavior:

- If `observability` is absent, no host-specific event sink is attached.
- If `observability false { ... }` is used, the block is ignored.
- Multiple `observability` directives for the same host accumulate event sinks.

Bundled provider-specific options:

### `provider console`

The bundled `console` provider takes no additional nested directives and writes supported observability events to Titanium's logs.

## Notes

- These directives are host-scoped rather than global.
- HTTP host directives are consumed at runtime, but they are not yet registered as per-protocol validators in the current loader path.
