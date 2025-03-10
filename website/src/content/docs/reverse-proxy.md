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