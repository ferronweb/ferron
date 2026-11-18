---
title: "Configuration: security and TLS"
description: "Certificate and key files, cipher suites, ECDH curves, TLS versions, and client certificate authentication (mTLS)."
---

TLS options live in a `tls { ... }` block and apply to every provider: `manual`, `acme`, `http`, and `local`. Ferron uses safe defaults when you omit a setting.

## Certificate and key

| Directive | Value     | Description                                                       |
| --------- | --------- | ----------------------------------------------------------------- |
| `cert`    | `<path>`  | PEM certificate chain. Leaf certificate first, then intermediates. |
| `key`     | `<path>`  | PEM private key.                                                  |

The `manual` provider requires both directives. Ferron must read both files, and the certificate and key must match. A mismatch makes every TLS handshake fail. Both paths accept environment variable interpolation.

```ferron
example.com {
    tls {
        provider manual
        cert /path/cert.pem
        key /path/key.pem
    }
}
```

## Cipher suites

`cipher_suite <suite>` adds one suite to the allowed list. Repeat the directive to add more suites. When you omit it, the rustls defaults apply.

```ferron
example.com {
    tls {
        provider manual
        cert /path/cert.pem
        key /path/key.pem
        cipher_suite TLS_AES_128_GCM_SHA256
        cipher_suite TLS_AES_256_GCM_SHA384
    }
}
```

### Supported cipher suites

| Suite                                           | Protocol | Key exchange  | Notes                               |
| ----------------------------------------------- | -------- | ------------- | ----------------------------------- |
| `TLS_AES_128_GCM_SHA256`                        | TLS 1.3  | Any           | Default in most deployments         |
| `TLS_AES_256_GCM_SHA384`                        | TLS 1.3  | Any           | Stronger encryption                 |
| `TLS_CHACHA20_POLY1305_SHA256`                  | TLS 1.3  | Any           | Software-optimized, good for mobile |
| `TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`       | TLS 1.2  | ECDHE + ECDSA | ECDSA certificate required          |
| `TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384`       | TLS 1.2  | ECDHE + ECDSA | ECDSA certificate required          |
| `TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256` | TLS 1.2  | ECDHE + ECDSA | ECDSA certificate required          |
| `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256`         | TLS 1.2  | ECDHE + RSA   | RSA certificate                     |
| `TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384`         | TLS 1.2  | ECDHE + RSA   | RSA certificate                     |
| `TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256`   | TLS 1.2  | ECDHE + RSA   | RSA certificate                     |

> [!note]
> TLS 1.2 cipher suites take effect only when `min_version` allows TLS 1.2.

## ECDH curves

`ecdh_curve <curve>` adds one key exchange group to the allowed list, in priority order. Repeat the directive to add more curves. When you omit it, the rustls defaults apply.

### Supported curves

| Curve            | Type                   | Notes                                              |
| ---------------- | ---------------------- | -------------------------------------------------- |
| `x25519`         | ECDH                   | Fast, widely supported, recommended default        |
| `secp256r1`      | ECDH                   | NIST P-256, required for some compliance standards |
| `secp384r1`      | ECDH                   | NIST P-384, higher security level                  |
| `x25519mlkem768` | Hybrid (ECDH + ML-KEM) | Post-quantum hybrid with `x25519`                  |
| `mlkem768`       | ML-KEM                 | Pure post-quantum KEM                              |

## TLS protocol versions

| Directive     | Values              | Default   |
| ------------- | ------------------- | --------- |
| `min_version` | `TLSv1.2`, `TLSv1.3` | `TLSv1.2` |
| `max_version` | `TLSv1.2`, `TLSv1.3` | `TLSv1.3` |

Omitting both keeps the TLS 1.2 to 1.3 range. Setting only `min_version` restricts the lower bound. Setting only `max_version` restricts the upper bound. Ferron rejects a `max_version` older than `min_version`.

```ferron
example.com {
    tls {
        provider manual
        cert /path/cert.pem
        key /path/key.pem
        min_version TLSv1.3
        max_version TLSv1.3
    }
}
```

## Client certificate authentication (mTLS)

| Directive              | Value                          | Default  | Description                                                  |
| ---------------------- | ------------------------------ | -------- | ------------------------------------------------------------ |
| `client_auth`          | `[bool]`                       | `false`  | Turn client certificate authentication on or off.            |
| `client_auth_required` | `[bool]`                       | `true`   | Require a valid client certificate when mTLS is enabled.      |
| `client_auth_ca`       | path, `system`, or `webpki`    | `webpki` | Source of trusted CA certificates for client verification.   |

`client_auth_ca` accepts these values:

- A file path loads the CA certificates in that PEM file.
- `system` uses the OS root store. It requires the `native-certs` feature.
- `webpki` uses the Mozilla root bundle.

```ferron
api.example.com {
    tls {
        provider manual
        cert /etc/ssl/api.example.com/cert.pem
        key /etc/ssl/api.example.com/key.pem

        min_version TLSv1.3
        max_version TLSv1.3

        cipher_suite TLS_AES_256_GCM_SHA384
        cipher_suite TLS_CHACHA20_POLY1305_SHA256

        ecdh_curve x25519
        ecdh_curve secp256r1

        client_auth
        client_auth_ca /etc/ssl/internal-ca/ca-bundle.pem
    }
}
```

Client certificate verification uses the same trust model as server-side TLS. The client chain must validate against the configured CA roots.

A `client_auth_ca` file can hold multiple CA certificates. Ferron loads all of them into the trust store.

The `system` trust store includes every OS-trusted root CA. Use it only when you want to accept client certificates from any publicly trusted CA, which is rarely right for mTLS. For internal mTLS, use a private CA and point `client_auth_ca` at the CA bundle file.

> [!tip]
> If client certificate handshakes fail:
>
> - Verify that the client certificate chain validates against the CA in `client_auth_ca`.
> - Check that the CA certificate file is valid PEM and has not expired.
> - With `client_auth_ca system`, check that the OS trusts the issuing CA.

## Metrics

| Metric                             | Type  | Attributes                                                                        | Description                                  |
| ---------------------------------- | ----- | --------------------------------------------------------------------------------- | -------------------------------------------- |
| `ferron.tls.certificate_not_after` | Gauge | `ferron.host`, `ferron.tls.provider` (provider name), `crypto.certificate.serial_number` | Certificate `notAfter` as Unix epoch seconds |

## See also

- [ACME automatic TLS](/docs/configuration/security/acme)
- [TLS session ticket keys](/docs/configuration/security/session-tickets)
- [OCSP stapling](/docs/configuration/security/ocsp)

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- `max_version TLSv1.2`: Disabling TLS 1.3 reduces security and performance. Allow TLS 1.3 unless legacy clients require TLS 1.2 only.
- `client_auth` with public trust store: Using `system` or `webpki` roots for mTLS client authentication trusts any certificate from the public PKI. Use a private CA bundle file for mTLS instead.
- `client_auth_required false`: This setting allows unauthenticated connections. If possible, deny unauthenticated connections for better access control.
- `ocsp` disabled: OCSP stapling improves TLS privacy, performance, and revocation behavior. Keep it enabled.
- `ticket_keys` without `auto_rotate`: Session ticket keys should rotate automatically in production to limit the impact of key compromise.
- `ticket_keys.max_keys` outside 2-5: The optimal range keeps enough old keys for rotation without interruption and without excessive retention.
- `ticket_keys.rotation_interval` longer than 24h: Rotate session ticket keys every 12-24 hours in production.
