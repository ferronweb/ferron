---
title: Getting started with Ferron
description: "A beginner guide to web servers and Ferron: when to serve static files, when to reverse proxy, and how to choose your first setup."
---

If this is your first time using a web server, start here.

Ferron can do two common jobs:

- Serve files directly (HTML/CSS/JS/images) from disk.
- Forward requests to an app server (reverse proxying).

You can also combine both in one config.

## Web server basics in 60 seconds

A web server listens for HTTP/HTTPS requests and returns responses.

- Static file serving - the server reads files from disk and sends them to the client.
- Reverse proxying - the server forwards requests to another server (for example Node.js, Python, Go, Java, PHP-FPM, or another API service), then returns that response to the client.

## Which setup should you choose?

Choose **static file serving** if:

- You have a static website, docs site, landing page, or built frontend files.
- You do not need app logic on each request.
- Your content mostly comes from files in a directory.

Choose **reverse proxying** if:

- You run an app process that generates responses dynamically.
- You already have a backend listening on a port/socket.
- You need Ferron in front for TLS, routing, caching, or observability.

Choose a **mixed setup** if:

- You serve frontend files and proxy API/app requests (for example `/api`).

## First configuration examples

### 1. Static file serving

```kdl
// Replace "example.com" with your domain name
example.com {
    root "/var/www/html" // Replace with your directory containing static files
}
```

### 2. Reverse proxying

```kdl
// Replace "example.com" with your domain name
example.com {
    proxy "http://localhost:3000/" // Replace with your backend URL
}
```

### 3. Mixed setup (static site + API)

```kdl
// Replace "example.com" with your domain name
example.com {
    location "/api" remove_base=#true {
        proxy "http://localhost:3000/api"
    }

    location "/" {
        root "/var/www/html"
    }
}
```

## Recommended path for first-time users

1. Install Ferron using one of these guides:
   - [Debian/Ubuntu](/docs/installation/debian)
   - [RHEL/Fedora](/docs/installation/rpm)
   - [Docker](/docs/installation/docker)
   - [Linux installer](/docs/installation/installer-linux)
   - [Windows installer](/docs/installation/installer-windows)
   - [Manual installation](/docs/installation/manual)
2. Pick one setup from this page.
3. Use a related use-case guide, such as [Static file serving](/docs/use-cases/static-file-serving), [Reverse proxying](/docs/use-cases/reverse-proxy), or [Web applications](/docs/use-cases/web-applications).
4. Start with a minimal config first, then add TLS/security/routing features.

## Common beginner mistakes

- Using `root` when you actually need `proxy` to a running app process.
- Proxying everything to an app when your content is only static files.
- Mixing static and API traffic without route separation (for example, missing `location "/api"`).
- Copying a complex configuration before verifying a minimal working setup.

## Next reads

- [Playground](/docs/playground) for trying Ferron quickly.
- [Configuration fundamentals](/docs/configuration/fundamentals) for KDL structure.
- [Automatic TLS](/docs/use-cases/automatic-tls) once your basic setup works.
