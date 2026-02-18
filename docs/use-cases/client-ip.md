---
title: Client IP
description: "Preserve and trust real client IPs in Ferron behind reverse proxies or load balancers using X-Forwarded-For and PROXY protocol."
---

If Ferron runs behind another proxy/load balancer, the direct peer address may be the proxy, not the real user IP. Configure trusted forwarding so logging, access control, and rate limiting use client IPs correctly.

## Behind HTTP reverse proxies (X-Forwarded-For)

```kdl
// Replace "example.com" with your domain name.
example.com {
    trust_x_forwarded_for
    proxy "http://127.0.0.1:3000/"
}
```

Use this when the upstream proxy sets `X-Forwarded-For`.

## Behind L4 load balancers (PROXY protocol)

```kdl
globals {
    protocol_proxy
}

example.com {
    proxy "http://127.0.0.1:3000/"
}
```

Use this when your front load balancer sends PROXY protocol headers.

## Pass normalized client IP to upstream apps

```kdl
example.com {
    proxy "http://127.0.0.1:3000/"
    proxy_request_header_replace "X-Real-IP" "{client_ip_canonical}"
}
```

## Notes and troubleshooting

- Enable `trust_x_forwarded_for` only when traffic is coming through a trusted proxy path.
- Enable `protocol_proxy` only when your frontend actually sends PROXY protocol headers.
- Validate behavior using access logs before relying on IP-based `allow`, `block`, or `limit` policies.
- For directive details, see [Configuration: security & TLS](/docs/configuration/security-tls), [Configuration: core directives](/docs/configuration/core-directives), and [Configuration: reverse proxying](/docs/configuration/reverse-proxying).
