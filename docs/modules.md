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
- _rproxy_ - this module enables reverse proxy functionality.
- _scgi_ - this module enables the support for connecting to SCGI servers.

The following modules are built into Ferron, but are disabled by default:

- _asgi_ - this module enables the support for ASGI web applications.
- _example_ - this module responds with "Hello World!" for "/hello" request paths.
- _wsgi_ - this module enables the support for WSGI web applications.
- _wsgid_ - this module enables the support for WSGI web applications running on a pre-forked worker pool.

## Module notes

### _asgi_ module

The _asgi_ module runs ASGI applications on a single worker process. Due to Python's GIL (Global Interpreter Lock), the performance might be lower than what it would be run on multiple worker processes.

This module expects the ASGI application to have `application` as the ASGI callback. If you're using some other callback name, you can create the file below (assuming that the callback name is `app` and the main application Python file is `app.py`):

```python
from app import app

application = app
```

This module requires that Ferron links to the Python library.

### _cache_ module

The _cache_ module is a simple in-memory cache module for Ferron that works with "Cache-Control" and "Vary" headers. The cache is shared across all threads.

It's recommended to load this module before modules that can be used to handle application data to ensure that the response from that module is cached. For example:

- to cache CGI responses, specify to load the _cache_ module before _cgi_ module
- to cache FastCGI responses, specify to load _cache_ module before _fcgi_ module
- to cache proxied responses, specify to load _cache_ module before _rproxy_ module
- to cache responses from the ASGI application, specify to load _cache_ module before _asgi_ module
- to cache responses from the WSGI application, specify to load _cache_ module before _wsgi_ or _wsgid_ module

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

### _rproxy_ module

The reverse proxy functionality is enabled when _proxyTo_ or _secureProxyTo_ configuration property is specified.

The following request headers are provided to the backend server:

- **X-Forwarded-Proto** - if the original request is encrypted, it's `"https"`, otherwise it's `"http"`.
- **X-Forwarded-Host** - the value of the _Host_ header from the original request
- **X-Forwarded-For** - the client's IP address

### _wsgi_ module

The _wsgi_ module runs WSGI applications on a single worker process. Due to Python's GIL (Global Interpreter Lock), the performance might be lower than what it would be run on multiple worker processes. If you are using Unix or a Unix-like system, it's recommended to use the _wsgid_ module instead.

This module expects the WSGI application to have `application` as the WSGI callback. If you're using some other callback name, you can create the file below (assuming that the callback name is `app` and the main application Python file is `app.py`):

```python
from app import app

application = app
```

This module requires that Ferron links to the Python library.

### _wsgid_ module

The _wsgid_ module runs WSGI applications on a pre-forked process pool. This module can be enabled only on Unix and a Unix-like systems. Additionaly, it's recommended to stop the processes in the process pool in addition to the main process, as the server will not automatically stop the processes in the process pool (except on Linux systems, where the processes in the process pool are automatically stopped when the server is stopped).

This module expects the WSGI application to have `application` as the WSGI callback. If you're using some other callback name, you can create the file below (assuming that the callback name is `app` and the main application Python file is `app.py`):

```python
from app import app

application = app
```

This module requires that Ferron links to the Python library.
