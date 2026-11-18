---
title: "Configuration: TLS session ticket keys"
description: "Stateless TLS session resumption with automatic key rotation and file-backed persistence."
---

TLS session tickets let clients resume an earlier TLS session without a full handshake. This reduces latency for returning clients. Ferron reads ticket keys from a file and can rotate them automatically.

## Static keys

```ferron
example.com {
    tls {
        provider manual
        cert cert.pem
        key key.pem
        ticket_keys {
            file session_tickets.keys
        }
    }
}
```

Ferron validates the key file and enables session tickets. It loads the keys once at startup.

> [!note]
> If the key file does not exist and `auto_rotate` is off, Ferron logs a warning and does not use session tickets.

## Automatic key rotation

```ferron
example.com {
    tls {
        provider manual
        cert cert.pem
        key key.pem
        ticket_keys {
            file session_tickets.keys
            auto_rotate
            rotation_interval "12h"
            max_keys 3
        }
    }
}
```

With `auto_rotate`, Ferron does the following:

- Creates the file with initial keys when the file does not exist.
- Rotates the keys every `rotation_interval`.
- Keeps up to `max_keys` keys, so old tickets remain readable.
- Writes each new key set to disk atomically.

> [!tip]
> Automatic rotation also works when several servers share the key file on a network volume.

## Options

| Parameter           | Type         | Default | Required | Description                   |
| ------------------- | ------------ | ------- | -------- | ----------------------------- |
| `file`              | `<string>`   | none    | Yes      | Path to the ticket key file   |
| `auto_rotate`       | `[<bool>]`   | `false` | No       | Enable automatic key rotation |
| `rotation_interval` | `<duration>` | `12h`   | No       | How often to rotate keys      |
| `max_keys`          | `<int>`      | `3`     | No       | Maximum keys to retain (2-5)  |

## Key file format

The file size must be a multiple of 80 bytes. Each 80-byte record contains:

- 16 bytes: key name
- 32 bytes: AES-256 key for encryption and decryption
- 32 bytes: HMAC-SHA256 key for authentication

### Generate a key file

When `auto_rotate` is off, generate the keys yourself:

```bash
# Generate a single 80-byte key
openssl rand 80 > session_tickets.keys

# Generate multiple keys (for rotation support)
openssl rand 80 > session_tickets.keys
openssl rand 80 >> session_tickets.keys
openssl rand 80 >> session_tickets.keys
```

> [!important]
> Use cryptographically secure randomness to generate keys.

### Rotate keys manually

```bash
# Rotate keys, keeping some previous keys from the old file
mv session_tickets.keys session_tickets.keys.old
openssl rand 80 > session_tickets.keys
# Append truncated old file, without some oldest keys
head -c -80 session_tickets.keys.old >> session_tickets.keys

# Reload the server
sudo kill -HUP $(pidof ferron)
```

### File permissions

The key file contains secret material. Set restrictive permissions:

```bash
chmod 600 session_tickets.keys
chown ferron:ferron session_tickets.keys
```

## How rotation works

Ferron encrypts with the newest key. Older keys remain available for decryption only. At each `rotation_interval`, Ferron generates a key and discards the oldest key. `max_keys` sets how many keys the file keeps.

Ferron writes the keys to the file, so they survive a server restart.

With `rotation_interval` set to 12 hours and `max_keys` set to 3:

| Time after start | Keys    | Encryption key |
| ---------------- | ------- | -------------- |
| 0 hours          | A, B, C | A              |
| 12 hours         | D, A, B | D              |
| 24 hours         | E, D, A | E              |
| 36 hours         | F, E, D | F              |

A ticket encrypted with key A at 0 hours remains valid until 24 hours, when Ferron discards key A.

## Security considerations

- Enable `auto_rotate` in production. Regular rotation limits the effect of a compromised key.
- Rotate keys every 12-24 hours.
- Do not expose the key file. Anyone with a key can decrypt TLS sessions. Do not log keys, make the file world-readable, or commit it to version control.
- Do not use predictable key values.

## See also

- [Security and TLS](/docs/configuration/security/tls): cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/configuration/security/acme): session tickets with ACME-obtained certificates
- [OCSP stapling](/docs/configuration/security/ocsp)
