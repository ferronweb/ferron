# TLS Crypto Settings and Client Certificates (mTLS)

## Overview

The `tls` block supports directives for tuning cipher suites, ECDH curves, protocol versions, and client certificate authentication (mTLS). These settings are handled by the selected TLS provider (e.g., `manual`) and are optional — safe defaults are used when omitted.

## Cipher Suites (`cipher_suite`)

Syntax:

```ferron
tls {
    provider manual
    cert "/path/cert.pem"
    key "/path/key.pem"
    cipher_suite TLS_AES_128_GCM_SHA256
    cipher_suite TLS_AES_256_GCM_SHA384
}
```

The `cipher_suite` directive is **repeatable**. Each occurrence adds one suite to the allowed list. When omitted, rustls defaults are used.

### Supported Cipher Suites

| Suite | Protocol | Key Exchange | Notes |
|-------|----------|-------------|-------|
| `TLS_AES_128_GCM_SHA256` | TLS 1.3 | Any | Default in most deployments |
| `TLS_AES_256_GCM_SHA384` | TLS 1.3 | Any | Stronger encryption |
| `TLS_CHACHA20_POLY1305_SHA256` | TLS 1.3 | Any | Software-optimized, good for mobile |
| `TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256` | TLS 1.2 | ECDHE + ECDSA | ECDSA certificate required |
| `TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384` | TLS 1.2 | ECDHE + ECDSA | ECDSA certificate required |
| `TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256` | TLS 1.2 | ECDHE + ECDSA | ECDSA certificate required |
| `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256` | TLS 1.2 | ECDHE + RSA | RSA certificate |
| `TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384` | TLS 1.2 | ECDHE + RSA | RSA certificate |
| `TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256` | TLS 1.2 | ECDHE + RSA | RSA certificate |

**Note:** TLS 1.2 cipher suites are only effective when `min_version` allows TLS 1.2.

## ECDH Curves (`ecdh_curve`)

Syntax:

```ferron
tls {
    provider manual
    cert "/path/cert.pem"
    key "/path/key.pem"
    ecdh_curve x25519
    ecdh_curve secp256r1
}
```

The `ecdh_curve` directive is **repeatable**. Each occurrence adds one key exchange group to the allowed list, in priority order. When omitted, rustls defaults are used.

### Supported Curves

| Curve | Type | Notes |
|-------|------|-------|
| `x25519` | ECDH | Fast, widely supported, recommended default |
| `secp256r1` | ECDH | NIST P-256, required for some compliance standards |
| `secp384r1` | ECDH | NIST P-384, higher security level |
| `x25519mlkem768` | Hybrid (ECDH + ML-KEM) | Post-quantum hybrid, experimental |
| `mlkem768` | ML-KEM | Pure post-quantum KEM, experimental |

## TLS Protocol Version (`min_version` / `max_version`)

Syntax:

```ferron
tls {
    provider manual
    cert "/path/cert.pem"
    key "/path/key.pem"
    min_version TLSv1.2
    max_version TLSv1.3
}
```

| Directive | Type | Default | Description |
|-----------|------|---------|-------------|
| `min_version` | `TLSv1.2`, `TLSv1.3` | `TLSv1.2` | Minimum allowed TLS version |
| `max_version` | `TLSv1.2`, `TLSv1.3` | `TLSv1.3` | Maximum allowed TLS version |

If both are omitted, the safe default range (TLS 1.2–1.3) is used. Setting only `min_version` restricts the lower bound; setting only `max_version` restricts the upper bound. An error is returned if `max_version` is older than `min_version`.

### Examples

**TLS 1.3 only (recommended for modern deployments):**

```ferron
tls {
    provider manual
    cert "/path/cert.pem"
    key "/path/key.pem"
    min_version TLSv1.3
    max_version TLSv1.3
}
```

**TLS 1.2 and 1.3 (backward compatibility):**

```ferron
tls {
    provider manual
    cert "/path/cert.pem"
    key "/path/key.pem"
    min_version TLSv1.2
}
```

## Client Certificate Authentication (mTLS)

The `manual` TLS provider supports mutual TLS (mTLS), requiring or optionally requesting client certificates signed by a trusted CA.

### Enabling mTLS

```ferron
tls {
    provider manual
    cert "/path/cert.pem"
    key "/path/key.pem"
    client_auth true
    client_auth_ca "/path/ca-cert.pem"
}
```

| Directive | Type | Default | Description |
|-----------|------|---------|-------------|
| `client_auth` | `<bool>` | `false` | Enables client certificate authentication. When `true`, clients **must** present a valid certificate. |
| `client_auth_ca` | `<string>` | `webpki` | Source of trusted CA certificates for verifying client certificates. See below. |

### `client_auth_ca` Values

| Value | Behavior | Feature Required |
|-------|----------|-----------------|
| `"/path/ca-cert.pem"` | Load a single CA certificate from the given PEM file. Multiple certs are supported if the file contains a chain. | — |
| `system` | Use the operating system's native root certificate store. | `native-certs` feature |
| `webpki` | Use the Mozilla root certificate bundle (webpki-roots). | `webpki-roots` feature |

### Optional vs. Required

By default, `client_auth true` makes client certificates **required** — the TLS handshake fails if the client does not present a valid certificate.

To offer client certificates **optionally** (clients may present one, but anonymous connections are also allowed), future versions may introduce a separate `client_auth_mode` directive. For now, use `client_auth true` for required mTLS.

### Example: Full mTLS Configuration

```ferron
api.example.com {
    tls {
        provider manual
        cert "/etc/ssl/api.example.com/cert.pem"
        key "/etc/ssl/api.example.com/key.pem"

        # TLS 1.3 only
        min_version TLSv1.3
        max_version TLSv1.3

        # Strong cipher suites
        cipher_suite TLS_AES_256_GCM_SHA384
        cipher_suite TLS_CHACHA20_POLY1305_SHA256

        # Preferred ECDH curves
        ecdh_curve x25519
        ecdh_curve secp256r1

        # Require client certificates signed by our internal CA
        client_auth true
        client_auth_ca "/etc/ssl/internal-ca/ca-bundle.pem"
    }
}
```

### Example: System Trust Store for Client Auth

```ferron
public.example.com {
    tls {
        provider manual
        cert "/etc/ssl/public.example.com/cert.pem"
        key "/etc/ssl/public.example.com/key.pem"

        # Optional: request client certs but allow anonymous (future)
        # For now, client_auth true = required
        client_auth true
        client_auth_ca system
    }
}
```

## Complete Reference for `tls` Block

All directives available inside a `tls { ... }` block:

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `provider` | `<string>` | TLS provider name (required) | — |
| `cert` | `<string>` | Path to a PEM certificate file | — |
| `key` | `<string>` | Path to a PEM private key file | — |
| `cipher_suite` | `<string>` | Add a cipher suite (repeatable) | rustls defaults |
| `ecdh_curve` | `<string>` | Add an ECDH curve (repeatable) | rustls defaults |
| `min_version` | `TLSv1.2`, `TLSv1.3` | Minimum TLS protocol version | `TLSv1.2` |
| `max_version` | `TLSv1.2`, `TLSv1.3` | Maximum TLS protocol version | `TLSv1.3` |
| `client_auth` | `<bool>` | Enable client certificate authentication | `false` |
| `client_auth_ca` | `<string>` | CA cert source for client auth | `webpki` |
| `ticket_keys` | `{ ... }` | Session ticket key management | Default ticketer |
| `ocsp` | `{ ... }` | OCSP stapling configuration | Enabled |

See also:

- [TLS Session Ticket Keys](./tls-session-tickets.md) — `ticket_keys` directive reference
- [OCSP Stapling](./ocsp-stapling.md) — `ocsp` directive reference

## Feature Flags

The `ferron-tls-manual` crate enables the following features by default:

| Feature | Description |
|---------|-------------|
| `native-certs` | Enables `client_auth_ca system` via `rustls-native-certs` |
| `webpki-roots` | Enables `client_auth_ca webpki` via `webpki-roots` |

If you are building a custom deployment without one of these features, the corresponding `client_auth_ca` mode will return an error at runtime.

## Security Considerations

### Cipher Suite Selection

- Prefer TLS 1.3 cipher suites (`TLS_AES_*`, `TLS_CHACHA20_*`) — they are simpler and avoid known TLS 1.2 weaknesses.
- Avoid mixing TLS 1.2 and 1.3 cipher suites unless you must support legacy clients.
- `TLS_CHACHA20_POLY1305_SHA256` is useful for devices without AES hardware acceleration.

### ECDH Curve Selection

- `x25519` is the recommended default: fast, secure, and widely supported.
- `secp256r1` may be needed for FIPS compliance or interoperability with systems that don't support Curve25519.
- Post-quantum curves (`x25519mlkem768`, `mlkem768`) are experimental — use only in testing environments.

### mTLS

- Client certificate verification uses the same trust model as server-side TLS: the client cert chain must validate against the configured CA roots.
- When `client_auth_ca` points to a file containing multiple CA certificates (a bundle), all of them are loaded into the trust store.
- The `system` trust store includes all OS-trusted root CAs — use it only when you want to accept client certificates from any publicly trusted CA (rarely the right choice for mTLS).
- For internal mTLS deployments, use a private CA and set `client_auth_ca` to the CA bundle file path.

## Troubleshooting

### "Invalid minimum/maximum TLS version"

The `min_version` or `max_version` value is not recognized. Ensure you use exactly `TLSv1.2` or `TLSv1.3`.

### "Maximum TLS version is older than minimum TLS version"

`max_version` must be equal to or newer than `min_version`.

### "native-certs feature not enabled" / "webpki-roots feature not enabled"

The `client_auth_ca` value requires a feature that is not compiled in. Enable the appropriate feature in your `Cargo.toml` or change the `client_auth_ca` value.

### Client certificate handshake failure

- Verify the client certificate chain validates against the CA specified in `client_auth_ca`.
- Check that the CA certificate file is a valid PEM and hasn't expired.
- If using `client_auth_ca system`, ensure the issuing CA is trusted by the OS.
