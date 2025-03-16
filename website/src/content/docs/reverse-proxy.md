---
title: Reverse proxy functionality
---

Configuring Ferron as a reverse proxy is straightforward - you just need to enable the "rproxy" module, and specify the backend server URL in "proxyTo" configuration property (or "secureProxyTo" for HTTPS connections). To configure Ferron as a reverse proxy, you can use the configuration below:

```yaml
# Example global configuration with reverse proxy
global:
  loadModules:
    - rproxy
  proxyTo: http://localhost:3000/ # Replace "http://localhost:3000" with the backend server URL
```

## Example: Ferron multiplexing to several backend servers

In this example, the `example.com` and `bar.example.com` domains point to a server running Ferron.

Below are assumptions made for this example:
- `https://example.com` is "main site", while `https://example.com/agenda` is hosting a calendar service.
- `https://foo.example.com` is passed to `https://saas.foo.net`
- `https://bar.example.com` is the front for an internal backend.

You can configure Ferron like this:

```yaml
global:
  secure: true
  cert: /path/to/certificate.crt # Replace "/path/to/certificate.crt" to the path to the TLS certificate
  key: /path/to/private.key # Replace "/path/to/private.key" to the path to the private key
  loadModules:
    - rproxy

hosts:
  - domain: example.com
    locations:
      - path: /agenda
        proxyTo: http://calender.example.net:5000
        # It proxies /agenda/example to http://calender.example.net:5000/agenda/example
      - path: "/" # Catch-all path
        proxyTo: http://localhost:3000/
  - domain: foo.example.com
    proxyTo: https://saas.foo.net
  - domain: bar.example.com
    proxyTo: http://backend.example.net:4000
```

For `http://calender.example.net:5000/agenda/example`, you will probably have to either configure the calendar service to strip 'agenda/' or configure URL rewriting in Ferron.
