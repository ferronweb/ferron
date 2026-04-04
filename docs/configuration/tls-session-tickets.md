# TLS Session Ticket Keys

## Overview

TLS session tickets enable **stateless session resumption**, allowing clients to resume previous TLS sessions without a full handshake. This improves performance and reduces latency for returning clients.

In Ferron 3, session ticket keys are managed through the `tls "manual"` provider, with support for key files that enable session resumption across server restarts and multiple server instances.

## Configuration

### Basic Usage

To enable session tickets with automatically generated random keys:

```ferron
tls {
    provider manual
    cert "cert.pem"
    key "key.pem"
}
```

This configuration enables session tickets with cryptographically random keys generated at startup. **Note**: Keys are not persisted across restarts with this configuration.

### With Shared Ticket Keys

To use shared ticket keys (recommended for production):

```ferron
tls {
    provider manual
    cert "cert.pem"
    key "key.pem"
    ticket_keys "session_tickets.keys"
}
```

The `ticket_keys` directive specifies a file containing pre-shared ticket keys. This enables:

- **Session resumption across restarts**: Survive configuration reloads and server restarts
- **Multi-instance support**: Share the same keys across multiple Ferron instances for cross-instance session resumption

## Key File Format

The ticket key file must follow a specific format:

- File size must be a multiple of **80 bytes**
- Each 80-byte record contains:
  - **16 bytes**: Key Name (unique identifier)
  - **32 bytes**: AES-256-GCM Key (encryption/decryption)
  - **32 bytes**: HMAC-SHA256 Key (authentication)

### Example: Generating a Key File

Use OpenSSL or similar tool to generate cryptographically secure keys:

```bash
# Generate a single 80-byte key
openssl rand 80 > session_tickets.keys

# Generate multiple keys (for rotation support)
openssl rand 80 > session_tickets.keys
openssl rand 80 >> session_tickets.keys
openssl rand 80 >> session_tickets.keys
```

**Important**: Keys must be generated using cryptographically secure randomness. Do not use predictable values.

### File Permissions

The ticket key file contains sensitive cryptographic material. Set restrictive permissions:

```bash
chmod 600 session_tickets.keys
chown ferron:ferron session_tickets.keys
```

The file should be readable only by the user running the Ferron process.

## Key Rotation

### Rotation Strategy

To rotate ticket keys without breaking existing sessions:

1. **Generate a new key**:
   ```bash
   openssl rand 80 > new_ticket_key.keys
   ```

2. **Prepend the new key** to existing keys (keep 1-2 old keys for overlap):
   ```bash
   # Assuming you have 2 existing keys in session_tickets.keys
   cat new_ticket_key.keys session_tickets.keys > temp.keys
   # Keep only first 3 keys (240 bytes)
   head -c 240 temp.keys > session_tickets.keys
   rm temp.keys new_ticket_key.keys
   ```

3. **Trigger a configuration reload**:
   ```bash
   # Send SIGHUP to the Ferron daemon
   kill -HUP <ferron-pid>
   ```

### How Rotation Works

When multiple keys are present in the file:

- **Encryption (issuing new tickets)**: Uses the **first** key in the file
- **Decryption (resuming sessions)**: Attempts **all** keys in the file

This design ensures:
- New tickets use the freshest key
- Existing sessions with older keys continue to work
- Gradual transition without breaking active sessions

### Recommended Key Count

- **Minimum**: 1 key (works, but no rotation capability)
- **Recommended**: 2-3 keys (allows smooth rotation)
- **Maximum**: Ferron loads only the first 3 keys; additional keys are ignored with a warning

## Current Implementation Status

### What's Implemented ✅

- Validation of ticket key file format (size must be multiple of 80 bytes)
- File existence and readability checks
- Session ticket enablement with rustls-generated random keys
- Logging of key file validation results
- Graceful error handling (invalid files prevent config reload)

### Current Limitations ⚠️

Due to limitations in rustls 0.23's public API:

- **Custom key loading**: The ticket key file is validated but not directly loaded into rustls
- **Random keys**: rustls internally generates random keys for the ticketer
- **Cross-instance resumption**: Requires waiting for rustls to expose the key loading API or implementing a custom `ProducesTickets` trait

This means:
- Session resumption **works** within a single server instance lifetime
- Keys are **not persisted** across restarts (despite the file being validated)
- Multi-instance session resumption is **not yet functional**

### Future Work

When rustls exposes the necessary API or when we implement a custom `ProducesTickets` trait:

- Full key file loading with direct integration into rustls
- Cross-instance session resumption with shared key files
- Key persistence across restarts

## Security Considerations

### Do's ✅

- Generate keys using cryptographically secure randomness (e.g., `openssl rand`)
- Set restrictive file permissions (`chmod 600`)
- Rotate keys regularly (recommended: every 24-48 hours)
- Keep 2-3 keys during rotation for smooth transition
- Store key files in secure locations (e.g., encrypted volumes, secret managers)

### Don'ts ❌

- **Never log key content** - Ferron never logs key bytes
- **Don't use predictable values** - No hardcoded or weak keys
- **Don't expose files** - Avoid world-readable permissions
- **Don't rotate all keys at once** - Keep old keys for overlap during rotation
- **Don't commit keys to version control** - Add to `.gitignore`

## Troubleshooting

### Common Errors

#### "TLS ticket key file is empty"

The key file exists but has zero bytes. Generate at least one 80-byte key.

#### "TLS ticket key file size (X) is not a multiple of 80 bytes"

The file size is incorrect. Ensure the file contains complete 80-byte records.

**Fix**:
```bash
# Regenerate the file with correct size
openssl rand 80 > session_tickets.keys
```

#### "No such file or directory"

The ticket key file path is incorrect or the file doesn't exist.

**Fix**: Verify the path is correct and accessible to the Ferron process.

#### Session resumption not working across restarts

This is expected with the current rustls limitation. The key file is validated but not loaded. Session resumption works within a single instance lifetime only.

**Workaround**: Wait for rustls to expose the custom key loading API, or implement a custom `ProducesTickets` trait.

### Debugging

Enable debug logging to see ticket key validation messages:

```bash
# In CLI
ferron run -v
```

You should see messages like:
```
[2026-04-04 12:35:27.042 INFO] TLS session ticket keys validated from /path/to/session_tickets.keys (1 keys loaded)
```

Or on error:
```
[2026-04-04 12:35:27.042 ERROR] Failed to load TLS session ticket keys from /path/to/session_tickets.keys: <error details>
```

## Integration with Config Reload

Ferron's configuration reload system ensures safe key rotation:

1. **Config reload triggered** (via `SIGHUP` or file change)
2. **TLS provider re-executes** with new configuration
3. **Ticket keys validated** from the specified file
4. **New `ServerConfig` created** with validated keys
5. **Atomic swap** via `ArcSwap` - zero downtime
6. **Old connections** continue with old config
7. **New connections** use new config

If key file validation fails during reload:
- **Reload is rejected** - old config is retained
- **Error is logged** - operator is notified
- **No service disruption** - existing connections continue normally

## Architecture

### Data Flow

```
session_tickets.keys file
    ↓
validate_ticket_keys_file() in types/tls
    ↓
TcpTlsManualProvider::execute() validates path
    ↓
rustls::crypto::aws_lc_rs::Ticketer::new() creates ticketer
    ↓
ServerConfig.ticketer is set
    ↓
TcpTlsManualResolver wraps ServerConfig
    ↓
TlsResolverRadixTree stores resolver
    ↓
ArcSwap atomic swap on config reload
```

### Module Responsibilities

- **`types/tls`**: Core ticket key validation and parsing utilities
- **`modules/tls-manual`**: Provider that integrates validation into TLS configuration
- **`core`**: Configuration reload infrastructure (no TLS-specific logic)

This design keeps TLS concerns within the TLS provider while enabling reusable validation logic across multiple TLS providers.

## References

- [RFC 5077: Transport Layer Security (TLS) Session Resumption](https://tools.ietf.org/html/rfc5077)
- [rustls documentation](https://docs.rs/rustls/)
- [Ferron TLS Provider Architecture](../README.md)
