---
title: Reverse proxy functionality
---

Configuring Ferron as a reverse proxy is straightforward - you just need to specify the backend server URL in "proxy" directive. To configure Ferron as a reverse proxy, you can use the configuration below:

```kdl
// Example global configuration with reverse proxy
* {
    proxy "http://localhost:3000/" // Replace "http://localhost:3000" with the backend server URL
}
```

## Example: Ferron multiplexing to several backend servers

In this example, the `example.com` and `bar.example.com` domains point to a server running Ferron.

Below are assumptions made for this example:

- `https://example.com` is "main site", while `https://example.com/agenda` is hosting a calendar service.
- `https://foo.example.com` is passed to `https://saas.foo.net`
- `https://bar.example.com` is the front for an internal backend.

You can configure Ferron like this:

```kdl
* {
    tls "/path/to/certificate.crt" "/path/to/private.key"
}

example.com {
    location "/agenda" {
        // It proxies /agenda/example to http://calender.example.net:5000/agenda/example
        proxy "http://calender.example.net:5000"
    }

    location "/" {
        // Catch-all path
        proxy "http://localhost:3000/"
    }
}

foo.example.com {
    proxy "https://saas.foo.net"
}

bar.example.com {
    proxy "http://backend.example.net:4000"
}
```

For `http://calender.example.net:5000/agenda/example`, you will probably have to either configure the calendar service to strip 'agenda/' or configure URL rewriting in Ferron.
