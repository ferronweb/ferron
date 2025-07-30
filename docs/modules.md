---
title: Server modules
---

You can extend Ferron with modules written in Rust.

The following modules are built into Ferron and are enabled by default:

- _cache_ - this module enables server response caching.
- _cgi_ - this module enables the execution of CGI programs.
- _fauth_ - this module enables authentication forwarded to the authentication server.
- _fcgi_ - this module enables the support for connecting to FastCGI servers.
- _fproxy_ - this module enables forward proxy functionality.
- _limit_ (Ferron 2.0.0-beta.2 and newer) - this module enables rate limits.
- _replace_ (Ferron 2.0.0-beta.2 and newer) - this module enables replacement of strings in response bodies.
- _rproxy_ - this module enables reverse proxy functionality.
- _scgi_ - this module enables the support for connecting to SCGI servers.
- _static_ (Ferron 2.0.0-beta.1 and newer) - this module enables static file serving.

Ferron also supports additional modules that can be enabled at compile-time.

Additional modules provided by Ferron are from these repositories:

- [ferron-modules-python](https://github.com/ferronweb/ferron-modules-python.git) - provides gateway interfaces (ASGI, WSGI) utilizing Python.
- [ferron-module-example](https://github.com/ferronweb/ferron-module-example.git) - responds with "Hello World!" for "/hello" request paths.

If you would like to use Ferron with additional modules, you can check the [compilation notes](https://github.com/ferronweb/ferron/blob/2.x/COMPILATION.md).

## Module notes

### _cache_ module

The _cache_ module is a simple in-memory cache module for Ferron that works with "Cache-Control" and "Vary" headers. The cache is shared across all threads.

### _cgi_ module

To run PHP scripts with this module, you may need to adjust the PHP configuration file, typically located at `/etc/php/<php version>/cgi/php.ini`, by setting the `cgi.force_redirect` property to 0. If you don't make this change, PHP-CGI will show a warning indicating that the PHP-CGI binary was compiled with `force-cgi-redirect` enabled. It is advisable to use directories outside of _cgi-bin_ for user uploads and downloads to prevent the _cgi_ module from mistakenly treating uploaded scripts with shebangs and ELF binary files as CGI applications, which could lead to issues such as malware infections, remote code execution vulnerabilities, or 500 Internal Server Errors.

### _fauth_ module

This module is inspired by [Traefik's ForwardAuth middleware](https://doc.traefik.io/traefik/middlewares/http/forwardauth/). If the authentication server replies with a 2xx status code, access is allowed, and the initial request is executed. If not, the response from the authentication server is sent back.

The following request headers are provided to the authentication server:

- **X-Forwarded-Method** - the HTTP method used by the original request
- **X-Forwarded-Proto** - if the original request is encrypted, it's `"https"`, otherwise it's `"http"`.
- **X-Forwarded-Host** - the value of the _Host_ header from the original request
- **X-Forwarded-Uri** - the original request URI
- **X-Forwarded-For** - the client's IP address

### _fcgi_ module

PHP-FPM may run on different user than Ferron, so you might need to set permissions for the PHP-FPM user.

If you are using PHP-FPM only for Ferron, you can set the `listen.owner` and `listen.group` properties to the Ferron user in the PHP-FPM pool configuration file (e.g. `/etc/php/8.2/fpm/pool.d/www.conf`).

### _fproxy_ module

If you are using the _fproxy_ module, then hosts on the local network and local host are also accessible from the proxy. You may block these using a firewall, if you don’t want these hosts to be accessible from the proxy.

### _limit_ module

This module uses a Token Bucket algorithm. The rate limitation is on per-IP address basis.

### _replace_ module

If you're using this module with static file serving, it's recommended to disable static file compression using `compressed #false`, otherwise the replacement wouldn't work.

### _rproxy_ module

The reverse proxy functionality is enabled when _proxyTo_ or _secureProxyTo_ configuration property is specified.

The following request headers are provided to the backend server:

- **X-Forwarded-Proto** - if the original request is encrypted, it's `"https"`, otherwise it's `"http"`.
- **X-Forwarded-Host** - the value of the _Host_ header from the original request
- **X-Forwarded-For** - the client's IP address
