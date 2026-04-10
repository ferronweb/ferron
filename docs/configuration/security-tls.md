---
title: "Configuration: security and TLS"
description: "Cipher suites, ECDH curves, TLS protocol versions, and client certificate authentication (mTLS)."
---

This page documents the TLS crypto directives available inside a `tls { ... }` block. These settings are optional — safe defaults are used when omitted.

## Directives

### Cipher suites

- `cipher_suite <suite: string>`
  - This directive specifies a cipher suite to add to the allowed list. Repeatable — each occurrence adds one suite. When omitted, rustls defaults are used. Default: rustls defaults

**Configuration example:**

```ferron
tls {
    provider manual
    cert "/path/cert.pem"
    key "/path/key.pem"
    cipher_suite TLS_AES_128_GCM_SHA256
    cipher_suite TLS_AES_256_GCM_SHA384
}
```

#### Supported cipher suites

| Suite | Protocol | Key exchange | Notes |
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

### ECDH curves

- `ecdh_curve <curve: string>`
  - This directive specifies an ECDH key exchange group to add to the allowed list, in priority order. Repeatable — each occurrence adds one curve. When omitted, rustls defaults are used. Default: rustls defaults

#### Supported curves

| Curve | Type | Notes |
|-------|------|-------|
| `x25519` | ECDH | Fast, widely supported, recommended default |
| `secp256r1` | ECDH | NIST P-256, required for some compliance standards |
| `secp384r1` | ECDH | NIST P-384, higher security level |
| `x25519mlkem768` | Hybrid (ECDH + ML-KEM) | Post-quantum hybrid, experimental |
| `mlkem768` | ML-KEM | Pure post-quantum KEM, experimental |

### TLS protocol version

- `min_version <version: string>`
  - This directive specifies the minimum allowed TLS version. Supported values: `TLSv1.2`, `TLSv1.3`. Default: `min_version TLSv1.2`
- `max_version <version: string>`
  - This directive specifies the maximum allowed TLS version. Supported values: `TLSv1.2`, `TLSv1.3`. Default: `max_version TLSv1.3`

**Configuration example — TLS 1.3 only:**

```ferron
tls {
    provider manual
    cert "/path/cert.pem"
    key "/path/key.pem"
    min_version TLSv1.3
    max_version TLSv1.3
}
```

If both are omitted, the safe default range (TLS 1.2–1.3) is used. Setting only `min_version` restricts the lower bound; setting only `max_version` restricts the upper bound. An error is returned if `max_version` is older than `min_version`.

### Client certificate authentication (mTLS)

- `client_auth [bool: boolean]`
  - This directive specifies whether client certificate authentication is enabled. When `true`, clients **must** present a valid certificate. Default: `client_auth false`
- `client_auth_ca <source: string>`
  - This directive specifies the source of trusted CA certificates for verifying client certificates. Supported values: a file path (`"/path/ca-cert.pem"`), `system` (OS native root store, requires `native-certs` feature), `webpki` (Mozilla root bundle, requires `webpki-roots` feature). Default: `client_auth_ca webpki`

**Configuration example — full mTLS:**

```ferron
api.example.com {
    tls {
        provider manual
        cert "/etc/ssl/api.example.com/cert.pem"
        key "/etc/ssl/api.example.com/key.pem"

        min_version TLSv1.3
        max_version TLSv1.3

        cipher_suite TLS_AES_256_GCM_SHA384
        cipher_suite TLS_CHACHA20_POLY1305_SHA256

        ecdh_curve x25519
        ecdh_curve secp256r1

        client_auth true
        client_auth_ca "/etc/ssl/internal-ca/ca-bundle.pem"
    }
}
```

Notes:

- Client certificate verification uses the same trust model as server-side TLS: the client cert chain must validate against the configured CA roots.
- When `client_auth_ca` points to a file containing multiple CA certificates (a bundle), all of them are loaded into the trust store.
- The `system` trust store includes all OS-trusted root CAs — use it only when you want to accept client certificates from any publicly trusted CA (rarely the right choice for mTLS).
- For internal mTLS deployments, use a private CA and set `client_auth_ca` to the CA bundle file path.

## Feature flags

The `tls-manual` crate enables the following features by default:

| Feature | Description |
|---------|-------------|
| `native-certs` | Enables `client_auth_ca system` via `rustls-native-certs` |
| `webpki-roots` | Enables `client_auth_ca webpki` via `webpki-roots` |

If you are building a custom deployment without one of these features, the corresponding `client_auth_ca` mode will return an error at runtime.

## Security considerations

- Prefer TLS 1.3 cipher suites (`TLS_AES_*`, `TLS_CHACHA20_*`) — they are simpler and avoid known TLS 1.2 weaknesses.
- `x25519` is the recommended default for ECDH curves: fast, secure, and widely supported.
- Post-quantum curves (`x25519mlkem768`, `mlkem768`) are experimental — use only in testing environments.

## Notes and troubleshooting

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

## See also

- [ACME automatic TLS](/docs/v3/configuration/tls-acme)
- [TLS session ticket keys](/docs/v3/configuration/tls-session-tickets)
- [OCSP stapling](/docs/v3/configuration/ocsp-stapling)
