---
title: Admin API
description: "Securely configure, access, and harden the Ferron admin API for server management."
---

The admin API is a built-in control plane that provides health checks, server status, configuration inspection, and remote reload capability. It runs on a separate HTTP listener from your web server and should be treated with the same security as a root shell on your server.

> [!warning]
> The admin API is **not encrypted** and has **no authentication** by default. You can enable bearer token authentication with the `auth_token` directive. All other security relies on network-level isolation (binding to `127.0.0.1` or using infrastructure firewalls).

## Secure default configuration

The recommended default binds the admin API to the loopback interface and enables only the endpoints you need:

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

This configuration is safe for most deployments because `127.0.0.1` is only reachable from the local machine.

## Health checks in containers and orchestration

If your server runs inside a container or is managed by an orchestrator (Docker, Kubernetes, systemd, etc.), you may need the `health` endpoint for liveness or readiness probes. The health endpoint returns `200 OK` while the server is running and `503 Service Unavailable` during shutdown.

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

With `127.0.0.1`, the health check works for any process running on the same host, including container orchestrators that inspect the container's network namespace.

## Accessing the admin API from a remote machine

Even with `auth_token` configured, you should never expose the admin API directly to untrusted networks. Use one of the following patterns instead.

### SSH tunneling (recommended)

SSH tunneling is the simplest and most secure way to access the admin API remotely:

```bash
ssh -L 8081:127.0.0.1:8081 admin@your-server
```

After the tunnel is established, open `http://127.0.0.1:8081` in your browser or use `curl`:

```bash
curl http://127.0.0.1:8081/health
curl http://127.0.0.1:8081/status
curl http://127.0.0.1:8081/config
curl -X POST http://127.0.0.1:8081/reload
curl http://127.0.0.1:8081/reload
curl http://127.0.0.1:8081/runtime
```

The SSH connection provides encryption and authentication, compensating for the lack of TLS in the admin API itself.

### Reverse proxy with authentication

For more complex setups, you can front the admin API with an authenticating reverse proxy:

```text
Remote user → Reverse Proxy (basic auth / OAuth) → 127.0.0.1:8081 (admin API)
```

Example proxy configuration (Ferron):

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

With this pattern, the admin API remains bound to `127.0.0.1`, and the proxy handles TLS termination and authentication.

## Managing endpoints by role

You can adjust the admin API configuration to the operator's role by enabling only the necessary endpoints.

### Read-only operator (monitoring only)

For operators who only need to check server status:

```ferron
{
    admin {
        listen "127.0.0.1:8081"

        health true
        status true
        config true
        reload false
        reload_get false
        runtime false
    }
}
```

### DevOps operator (full control)

For operators who need to manage the server:

```ferron
{
    admin {
        listen "127.0.0.1:8081"

        health true
        status true
        config true
        reload true
        reload_get true
        runtime true
    }
}
```

### Minimal operator (health check only)

For deployments where only health monitoring is needed:

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

## Hardening checklist

Use this checklist to verify your admin API is properly secured:

- [ ] The `listen` address is set to `127.0.0.1` (or another loopback address).
- [ ] Unnecessary endpoints are disabled (`status false`, `config false`, `reload false`, `reload_get false`, `runtime false`).
- [ ] No firewall rule allows external traffic to the admin port.
- [ ] Remote access uses SSH tunneling or an authenticating reverse proxy.
- [ ] The admin API is **never** bound to `0.0.0.0` or a public IP address.
- [ ] If the server is in a container, the admin port is not published to the host network.
- [ ] Observability sinks are configured to log admin API requests for auditing.

## Troubleshooting

### "Connection refused" when accessing the admin API locally

Verify that the admin listener is actually running and bound to the expected address:

```bash
ss -tlnp | grep 8081
```

If nothing is listed, check Ferron's logs for startup errors. The admin block must be present in your configuration for the listener to start.

### "Connection refused" from a remote machine

The admin API is not designed for remote access. Use SSH tunneling or a reverse proxy as described in [Accessing the admin API from a remote machine](#accessing-the-admin-api-from-a-remote-machine).

### `GET /reload` returns an error

The `GET /reload` endpoint requires `reload_get true` in the admin configuration. If it returns an error, check that the endpoint is enabled.

### `POST /reload` returns an error

The `POST /reload` endpoint requires `reload true` in the admin configuration. If it returns an error, check that the endpoint is enabled and that the new configuration file is valid. Use `ferron validate -c ferron.conf` before reloading to catch configuration errors.

### `GET /config` shows redacted values

Sensitive values (TLS keys, passwords, tokens, bearer credentials, and htpasswd entries) are automatically redacted and replaced with `"[redacted]"`. This is intentional and cannot be disabled. The redacted configuration is still useful for auditing routing rules, upstream addresses, and host configuration.

### Admin API stops responding after a configuration change

The admin listener is gracefully shut down and restarted during every configuration reload. If the new configuration is valid, a fresh listener starts immediately. If the admin block is removed from the new configuration, the listener is permanently stopped.

### Accidentally bound to `0.0.0.0`

If you have bound the admin API to `0.0.0.0`, change it to `127.0.0.1` immediately and reload:

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

Then verify the change took effect:

```bash
ss -tlnp | grep 8081
```

The output should show `127.0.0.1:8081`, not `0.0.0.0:8081`.

## See also

- [Core directives](/docs/v3/configuration/server/core-directives) — full admin block configuration reference.
- [Security and TLS](/docs/v3/configuration/security/tls) — TLS configuration for your web server hosts.
- [Observability & logging](/docs/v3/configuration/observability/logging) — configure observability sinks to audit admin API access.
- [Reverse proxying](/docs/v3/use-cases/traffic/reverse-proxy) — front the admin API with an authenticating reverse proxy.
