---
title: FIPS-certified cryptography
description: "Run Ferron with FIPS-certified cryptography from AWS-LC, and learn which algorithms FIPS builds allow."
---

FIPS builds use FIPS-certified cryptography from AWS-LC for all TLS and password hashing. Ferron publishes FIPS artifacts with a `+fips` suffix, and FIPS Docker images with a `-fips` tag suffix. A FIPS build prints `This build is configured to use FIPS-certified cryptography.` in its version output.

> [!note]
> This page states the self-audited compliance status of Ferron FIPS builds. The certified cryptographic module is AWS-LC. This self-audit is not a CMVP certificate for Ferron itself.

## When to use FIPS builds

Use a FIPS build when your environment requires FIPS 140-3 validated cryptography. If your environment has no such requirement, use a standard build. Standard builds support more algorithms, such as Argon2 password hashes and ChaCha20 cipher suites.

## Get a FIPS build

Build from source with the `fips` feature:

```bash
cargo build --release --features=fips
```

Or use the build tooling:

```bash
just build fips=true
./cross-build/build.sh <target> --fips
```

For Docker, set the `FIPS` build argument to `1`. See [Docker](/docs/installation/docker).

## Validated platforms

Ferron publishes FIPS artifacts for these targets:

| Target                       | Notes                            |
| ---------------------------- | -------------------------------- |
| `x86_64-unknown-linux-gnu`   | Includes Debian and RPM packages |
| `x86_64-unknown-linux-musl`  |                                  |
| `aarch64-unknown-linux-gnu`  | Includes Debian and RPM packages |
| `aarch64-unknown-linux-musl` |                                  |
| `x86_64-pc-windows-msvc`     | Includes installer and archive   |
| `x86_64-apple-darwin`        |                                  |
| `aarch64-apple-darwin`       |                                  |

FIPS Docker images cover `linux/amd64` and `linux/arm64`.

> [!important]
> FIPS 140-3 validation applies only within the operating environments in the AWS-LC FIPS security policy. Check the AWS-LC security policy before you deploy a target in a regulated environment.

## Allowed algorithms in FIPS builds

FIPS builds restrict configuration to approved algorithms. Non-approved selections fail closed. Ferron returns an error instead of silently downgrading.

### TLS protocol versions

FIPS builds support TLS 1.2 and TLS 1.3 only. Older versions are not available in any build. See [Security and TLS](/docs/configuration/security/tls).

### TLS cipher suites

FIPS builds allow only AES-GCM suites:

- `TLS_AES_128_GCM_SHA256`
- `TLS_AES_256_GCM_SHA384`
- `TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`
- `TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384`
- `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256`
- `TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384`

ChaCha20-Poly1305 suites are not approved. If you configure only ChaCha20 suites in a FIPS build, the server reports an error at startup.

### TLS key exchange groups

FIPS builds allow only NIST curves:

- `secp256r1`
- `secp384r1`

`x25519`, `x25519mlkem768`, and `mlkem768` are not approved. If you configure only these groups in a FIPS build, the server reports an error at startup.

### TLS session tickets

Session ticket encryption uses AES-256-CBC with HMAC-SHA256 authentication in Encrypt-then-MAC mode. Both are approved. Ticket keys come from the operating system random source. See [TLS session ticket keys](/docs/configuration/security/session-tickets).

### OCSP stapling

FIPS builds use SHA-256 or stronger for OCSP request and response hashing. SHA-1 requests and the SHA-1 fallback are disabled. OCSP response signatures must use RSA (2048-bit or larger) or ECDSA on NIST curves with SHA-256, SHA-384, or SHA-512. RSA-SHA1, Ed25519, and secp256k1 signatures are rejected. See [OCSP stapling](/docs/configuration/security/ocsp).

### Password hashing

FIPS builds verify only PBKDF2 hashes with approved message digests:

- `$pbkdf2-sha256$`
- `$pbkdf2-sha384$`
- `$pbkdf2-sha512$`

Argon2 and scrypt hashes are rejected. Password salts come from the operating system random source. The `ferron-passwd` FIPS utility generates `$pbkdf2-sha256$` hashes with 600,000 iterations. If you migrate existing credentials to a FIPS build, re-hash Argon2 and scrypt passwords with PBKDF2 first. See [HTTP basic authentication](/docs/configuration/security/basic-auth).

> [!warning]
> In a FIPS build, users with Argon2 or scrypt hashes cannot log in. The server rejects those hashes by design. Plan credential migration before you switch builds.

## Out of scope algorithms

FIPS builds still contain non-approved hashes and random generators for non-cryptographic purposes. These are not cryptography and sit outside FIPS scope:

- xxHash for cache keys, ETags, configuration hashes, and trace sampling.
- Hash-map hashers for in-memory tables, load balancer rings, and connection dispatch.
- Non-security random values for OCSP refresh jitter, load balancer selection, trace identifiers, and multipart boundaries, as well as non-security-related uses for QUIC and DNS resolution.
- Base64 and hex encodings for configuration and protocol formatting.
- SHA-1 hashes (deprecated for cryptographic use) for Redis/Valkey Lua script hashing. This also uses non-FIPS-certified SHA-1 implementation.

## Known limitations

- DNS TSIG key algorithms are not restricted in FIPS builds. `HMAC-MD5` and `HMAC-SHA1` remain selectable for DNS updates. Do not use them in a FIPS deployment. Use `HMAC-SHA256` or stronger.
- The QUIC stateless-reset key comes from the QUIC library default. It uses HMAC-SHA256, but the key bytes come from a non-validated random generator instead of the certified module. This affects only connection-reset authentication, not traffic encryption.
- Certificate issuance helpers assemble X.509 certificates outside the validated module boundary. Key generation and signing still run through AWS-LC.
