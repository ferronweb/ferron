---
title: Getting started with Ferron 3
description: "Choose your first Ferron 3 setup, start from a minimal config, and move on to the right installation and use-case guides."
---

If this is your first time setting up Ferron, start here.

Ferron usually does one of two jobs:

- Serve files directly from disk.
- Forward requests to an app server.

You can also combine both in one host block.

## Web server basics in 60 seconds

A web server listens for HTTP and HTTPS requests and returns responses.

- Static file serving means Ferron reads files from disk and sends them to the browser.
- Reverse proxying means Ferron forwards a request to another server and returns that response to the browser.
- Mixed setups are common when you serve a frontend and proxy API requests.

## Which setup should you choose?

Choose **static file serving** if:

- You have a static website, docs site, landing page, or built frontend assets.
- You do not need app logic on every request.
- Your content mostly lives in a directory on disk.

Choose **reverse proxying** if:

- You already have an app process listening on a port or socket.
- You want Ferron in front for TLS, routing, access control, or observability.
- You are exposing a backend service such as Node.js, Python, Go, Java, or an API gateway.

Choose a **mixed setup** if:

- You serve a frontend and proxy API requests.
- You want static assets at `/` and app traffic under `/api`.
- You need one site to combine file serving and upstream routing.

## First configuration examples

### Static file serving

```ferron
example.com {
    root /var/www/html
}
```

### Reverse proxying

```ferron
example.com {
    proxy http://localhost:3000
}
```

### Mixed setup

```ferron
example.com {
    location /api {
        proxy http://localhost:3000
    }

    location / {
        root /var/www/html
    }
}
```

Ferron 3 strips the matched `location` prefix before the next stage runs, so the backend sees the path after `/api`.

## Recommended path for first-time users

1. Install Ferron with the guide that matches your environment:
   - [Linux installer](/docs/v3/installation/installer)
   - [Debian/Ubuntu](/docs/v3/installation/debian)
   - [RHEL/Fedora](/docs/v3/installation/rpm)
   - [Docker](/docs/v3/installation/docker)
   - [Windows installer](/docs/v3/installation/windows)
   - [Manual installation](/docs/v3/installation/archive)
   - [Build from source with default modules](/docs/v3/installation/source-default-modules)
   - [Build from source with custom modules](/docs/v3/installation/source-custom-modules)
2. Start with the smallest working config from this page.
3. Pick a deeper guide once the basic setup works:
   - [Static file serving](/docs/v3/use-cases/static-file-serving)
   - [Reverse proxying](/docs/v3/use-cases/reverse-proxy)
   - [Automatic TLS](/docs/v3/use-cases/automatic-tls)
4. Validate your config before restarting or reloading Ferron with `ferron validate -c ferron.conf`.

## Common beginner mistakes

- Using `root` when you actually need `proxy` for a running application.
- Proxying everything to an app when your content is mostly static files.
- Forgetting that `location` strips the matched prefix in Ferron 3.
- Copying a complex configuration before verifying a minimal working setup.

## Notes and troubleshooting

- If you are testing locally, start with a single host block and one directive at a time.
- If a mixed setup behaves oddly, confirm which `location` block matches first and remember that Ferron strips the matched prefix.
- If validation fails, fix the reported config error before trying to restart the server.
