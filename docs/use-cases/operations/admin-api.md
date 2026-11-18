---
title: Admin API
description: "Configure the admin listener, enable only the endpoints you need, and access it over SSH or an authenticating proxy."
---

The admin API runs on a separate HTTP listener. It provides health checks, server status, configuration inspection, and remote reload. Treat it with the same care as a root shell.

> [!warning]
> The admin API is not encrypted and has no authentication by default. Enable `auth_token` for bearer token authentication, and keep the listener on a loopback address or behind a firewall. Do not expose it to untrusted networks.

## Secure default configuration

```ferron
{
    admin {
        listen "127.0.0.1:8081"

        health true
        status false
        config false
        reload true
        reload_get true
        runtime true
    }
}
```

`127.0.0.1` is reachable only from the local machine. Enable only the endpoints that the operator needs.

| Endpoint    | Method     | Enable with  | Purpose                                             |
| ----------- | ---------- | ------------ | --------------------------------------------------- |
| `/health`   | `GET`      | `health`     | Liveness or readiness probe.                        |
| `/status`   | `GET`      | `status`     | Server status.                                      |
| `/config`   | `GET`      | `config`     | Running configuration with secrets redacted.        |
| `/reload`   | `POST`     | `reload`     | Reload the configuration.                           |
| `/reload`   | `GET`      | `reload_get` | Reload the configuration with a GET request.        |
| `/runtime`  | `GET`      | `runtime`    | Runtime information.                                |

`/config` replaces TLS keys, passwords, tokens, bearer credentials, and htpasswd entries with `"[redacted]"`. You cannot disable the redaction.

## Use the health endpoint

For a container or orchestrator probe, enable only `health`:

```ferron
{
    admin {
        listen "127.0.0.1:8081"

        health true
        status false
        config false
        reload false
        reload_get false
        runtime false
    }
}
```

A running server returns `200 OK`. During shutdown, the endpoint returns `503 Service Unavailable`.

With `127.0.0.1`, the probe works for any process in the same host or container network namespace.

## Access from a remote machine

Do not expose the admin API directly. Use one of these patterns instead.

### SSH tunnel

```bash
ssh -L 8081:127.0.0.1:8081 admin@your-server
```

Then call the API through the tunnel:

```bash
curl http://127.0.0.1:8081/health
curl http://127.0.0.1:8081/status
curl http://127.0.0.1:8081/config
curl -X POST http://127.0.0.1:8081/reload
curl http://127.0.0.1:8081/reload
curl http://127.0.0.1:8081/runtime
```

SSH supplies encryption and authentication. The admin API needs neither on the loopback address.

### Authenticating reverse proxy

A reverse proxy can terminate TLS and authenticate the operator:

```text
Remote user -> Reverse proxy (Basic Auth or OAuth) -> 127.0.0.1:8081
```

```ferron
admin.example.com {
    proxy http://127.0.0.1:8081

    basic_auth {
        realm "Admin API"
        users {
            alice "$argon2id$v=19$m=19456,t=2,p=1$..."
        }
    }
}
```

Keep the admin API bound to `127.0.0.1`. The proxy handles the public side.

## Hardening checklist

- Bind `listen` to `127.0.0.1` or another loopback address.
- Disable unused endpoints with `status false`, `config false`, `reload false`, `reload_get false`, and `runtime false`.
- Do not allow external traffic to the admin port in a firewall.
- Use an SSH tunnel or an authenticating proxy for remote access.
- Do not bind the admin API to `0.0.0.0` or a public address.
- Do not publish the admin port to the host network in a container.

## Troubleshooting

### Connection refused on the local machine

Check the listener and the address:

```bash
ss -tlnp | grep 8081
```

If the command prints nothing, read the Ferron startup logs. The `admin` block must exist for the listener to start.

### Connection refused from a remote machine

The admin API is not designed for remote access. Use an SSH tunnel or a reverse proxy.

### `GET /reload` returns an error

Set `reload_get true` in the `admin` block.

### `POST /reload` returns an error

Set `reload true`. Validate the new configuration with `ferron validate -c ferron.conf` before you reload.

### `/config` shows redacted values

Ferron always redacts sensitive values. The remaining output still shows routing rules, upstream addresses, and host configuration.

### The admin API stops after a configuration change

Ferron stops and restarts the admin listener on every reload. A valid configuration starts a new listener immediately. If you remove the `admin` block, the listener stays off.

### The admin API is bound to `0.0.0.0`

Change the address and reload:

```ferron
{
    admin {
        listen "127.0.0.1:8081"
        health true
        status false
        config false
        reload true
        reload_get false
        runtime false
    }
}
```

Then check the result:

```bash
ss -tlnp | grep 8081
```

The output must show `127.0.0.1:8081`.

## See also

- [Core directives](/docs/configuration/server/core-directives): full `admin` block reference.
- [Security and TLS](/docs/configuration/security/tls): TLS for public hosts.
- [Logging and observability](/docs/use-cases/operations/logging-observability): audit admin API access.
- [Reverse proxying](/docs/use-cases/traffic/reverse-proxy): front the admin API with an authenticating proxy.
