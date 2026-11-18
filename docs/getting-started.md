---
title: Getting started with Ferron 3
description: "Choose a static, reverse proxy, or mixed setup, then start from a minimal Ferron 3 configuration."
---

Ferron serves files from disk, forwards requests to an application server, or does both in one host block.

## Choose a setup

| Setup         | Use it when                                        | Main directive |
| ------------- | -------------------------------------------------- | -------------- |
| Static files  | You have a built site or static assets on disk.    | `root`         |
| Reverse proxy | An application listens on a port or socket.        | `proxy`        |
| Mixed         | A frontend serves `/` and an app serves `/api`.    | `location`     |

## Minimal configurations

### Static files

```ferron
example.com {
    root /var/www/html
}
```

### Reverse proxy

```ferron
example.com {
    proxy http://localhost:3000
}
```

### Mixed

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

Ferron strips the matched `location` prefix before the next stage runs. A request to `/api/users` reaches the backend as `/users`.

## Next steps

1. Install Ferron with the guide for your platform:
   - [Linux installer](/docs/installation/linux/installer)
   - [Debian/Ubuntu](/docs/installation/linux/debian-ubuntu)
   - [RHEL/Fedora](/docs/installation/linux/rhel-fedora)
   - [Docker](/docs/installation/docker)
   - [Windows installer](/docs/installation/windows)
   - [Manual installation](/docs/installation/manual-installation)
   - [Build from source with default modules](/docs/installation/source/default-modules)
   - [Build from source with custom modules](/docs/installation/source/custom-modules)
2. Start with one of the configurations above.
3. Validate the configuration before you reload or restart Ferron:

   ```bash
   ferron validate -c ferron.conf
   ```

4. Continue with the guide for your setup:
   - [Static file serving](/docs/use-cases/content/static-files)
   - [Reverse proxying](/docs/use-cases/traffic/reverse-proxy)
   - [Automatic TLS](/docs/use-cases/https/automatic-tls)

## Common mistakes

- Use `root` when the content exists on disk. Use `proxy` when an application serves the request.
- Do not proxy all traffic when most of the site is static.
- Remember that `location` strips the matched prefix.

> [!tip]
> Test one host block and one directive at a time. If validation fails, fix the reported error before you restart the server.
