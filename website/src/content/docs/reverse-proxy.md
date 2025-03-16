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

Ferron multiplexer to several backends
--------------------------------------

DNS points for `example.com` and `bar.example` to one _ferron instance_.

https://example.com  is "main site", https://example.com/agenda  is calender stuff.

https://foo.example.com goes to https://saas.foo.net

https://bar.example.com is _front_ for an _internal backend_.


```yaml
global:
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
    secureProxyTo: https://saas.foo.net
  - domain: bar.example.com
    proxyTo: http://backend.example.net:4000
```

For the `to http://calender.example.net:5000/agenda/example`
you probably have tell the calender back to strip 'agenda/'
or you will have to configure URL rewriting.
