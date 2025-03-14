---
title: Server configuration properties
---

Ferron can be configured in the `ferron.yaml` file. Below is the description of configuration properties for this server.

## Global-only configuration properties

- **port** (_u16_ or _String_)
  - HTTP port or address-port combination for the server to listen. This is the primary port on which the server will accept incoming HTTP connections. Default: `80`
- **sport** (_u16_ or _String_)
  - HTTPS port or address-port combination for the server to listen. This is the primary port on which the server will accept incoming HTTPS connections. Default: `443`
- **secure** (_bool_)
  - Option to enable HTTPS. When set to `true`, the server will use HTTPS for secure communication. Default: `false`
- **http2Settings** (_Object_)
  - HTTP/2 settings. This object contains various settings related to HTTP/2 protocol configuration. Default: None
  - **Sub-properties**:
    - **initialWindowSize** (_u32_)
      - Initial window size for HTTP/2. This setting controls the initial flow control window size for HTTP/2 connections. Default: None
    - **maxFrameSize** (_u32_)
      - Maximum frame size for HTTP/2. This setting determines the largest frame payload that the server will accept. Default: None
    - **maxConcurrentStreams** (_u32_)
      - Maximum number of concurrent streams for HTTP/2. This setting limits the number of concurrent streams that can be open at any given time. Default: None
    - **maxHeaderListSize** (_u32_)
      - Maximum header list size for HTTP/2. This setting controls the maximum size of the header list that the server will accept. Default: None
    - **enableConnectProtocol** (_bool_)
      - Enable the HTTP/2 CONNECT protocol. When set to `true`, the server will support the CONNECT method for tunneling TCP connections. Default: `false`
- **logFilePath** (_String_)
  - Path to the log file. This setting specifies the file path where the server will write its logs. The logs written by Ferron use Combined Log Format. Default: None
- **errorLogFilePath** (_String_)
  - Path to the error log file. This setting specifies the file path where the server will write its error logs. Default: None
- **enableHTTP2** (_bool_)
  - Option to enable HTTP/2. When set to `true`, the server will support the HTTP/2 protocol. Default: `false`
- **cert** (_String_)
  - Path to the TLS certificate. This setting specifies the file path to the TLS certificate used for HTTPS connections. Default: None
- **key** (_String_)
  - Path to the private key. This setting specifies the file path to the private key associated with the TLS certificate. Default: None
- **sni** (_Object_)
  - SNI certificate and key data. This object contains the certificate and key data for Server Name Indication (SNI), which allows multiple SSL certificates to be used on the same IP address. Default: None
  - **Sub-properties**:
    - **cert** (_String_)
      - Path to the SNI certificate. This setting specifies the file path to the SNI certificate. Default: None
    - **key** (_String_)
      - Path to the SNI private key. This setting specifies the file path to the private key associated with the SNI certificate. Default: None
- **loadModules** (_Array&lt;String&gt;_)
  - Modules to load. This setting specifies an array of modules that the server should load at startup. First module in the array is loaded first. Default: None
- **useClientCertificate** (_bool_)
  - Option to require client to provide its certificate. When set to `true`, the server will require clients to present a valid certificate for authentication. Default: `false`
- **cipherSuite** (_Array&lt;String&gt;_)
  - A list of cipher suites. This setting specifies an array of cipher suites that the server will support for encrypted connections. Default: None
- **ecdhCurve** (_Array&lt;String&gt;_)
  - A list of ECDH curves. This setting specifies an array of elliptic curves that the server will support for ECDH key exchanges. Default: None
- **tlsMinVersion** (_String_)
  - Minimum TLS version (TLSv1.2 or TLSv1.3). This setting specifies the minimum version of TLS that the server will accept. Default: `"TLSv1.2"`
- **tlsMaxVersion** (_String_)
  - Maximum TLS version (TLSv1.2 or TLSv1.3). This setting specifies the maximum version of TLS that the server will accept. Default: `"TLSv1.3"`
- **disableNonEncryptedServer** (_bool_)
  - Option to disable the HTTP server if the HTTPS server is running. When set to `true`, the server will only accept HTTPS connections and will disable the HTTP server. Default: `false`
- **blocklist** (_Array&lt;String&gt;_)
  - IP block list. This setting specifies an array of IP addresses that the server will block from accessing its services. The block list will only work with non-forward proxy requests. Default: None
- **enableOCSPStapling** (_bool_)
  - Option to enable OCSP stapling. When set to `true`, the server will use OCSP stapling to provide certificate revocation status to clients. Certificates with `Must-Staple` extension will not work with automatic TLS enabled. Default: `false`
- **environmentVariables** (_Object_)
  - Environment variables. This object contains environment variables that the server will use during operation. Default: None
- **enableAutomaticTLS** (_bool_; Project Karpacz 0.7.0 and newer)
  - Option to enable automatic TLS through Let's Encrypt. The automatic TLS will use an TLS-ALPN-01 ACME challenge. The domain names for the certificate will be extracted from the host configuration (wildcard domains are ignored, since TLS-ALPN-01 ACME challenge doesn't support them). The automatic TLS will work when the HTTPS port is set to `443`. Default: `false`
- **automaticTLSContactEmail** (_String_; Project Karpacz 0.7.0 and newer)
  - The email address used by automatic TLS for an account in Let's Encrypt. This email address can be used to send notifications by Let's Encrypt. Default: None
- **automaticTLSContactCacheDirectory** (_String_; Project Karpacz 0.7.0 and newer)
  - The path to the directory used by automatic TLS to store cache data, such as cached certificates. Default: None
- **automaticTLSLetsEncryptProduction** (_bool_; Project Karpacz 0.7.0 and newer)
  - Option to enable production Let's Encrypt ACME endpoint. If set to `false`, the staging Let's Encrypt ACME endpoint will be used. Default: `true`
- **loadBalancerHealthCheckWindow** (_u32_; _rproxy_ module; Ferron 1.0.0-beta3 and newer)
  - A window (in milliseconds) between each failed connection report made by a load balancer. Default: `5000`
- **timeout** (_u32_ or `null`; Ferron 1.0.0-beta3 and newer)
  - A maximum time (in milliseconds) for server to process the request, after which the server resets the connection. If set to `null`, the timeout is disabled. It's not recommended to disable the timeout, as disabling it may leave the server vulnerable to Slow HTTP attacks. Default: `300000`

## Host configuration properties

- **locations** (_Array&lt;Object&gt;_; Ferron 1.0.0-beta2 and newer)
  - The list of locations specified for a specific host. The sub-properties of this property will be merged in combined server configuration. The URLs will not be rewritten. To rewrite the URLs, configure the URL rewrite map in location configuration. Default: None

## Location configuration properties

- **path** (_String_; Ferron 1.0.0-beta2 and newer)
  - The path specified for a location. The location is matched against URL-decoded request URL. If it is the same, or the decoded request URL are levels above the specified path, the location configuration will be used. Default: None

## Global, host & location configuration properties

- **domain** (_String_)
  - The domain name of a host. This setting specifies the domain name associated with the host. Default: None
- **ip** (_String_)
  - The IP address of a host. This setting specifies the IP address associated with the host. Default: None
- **serverAdministratorEmail** (_String_)
  - Server administrator's email address. This setting specifies the email address of the server administrator, which may be used for contact purposes. Default: None
- **customHeaders** (_Object_)
  - Custom HTTP headers. This object contains custom HTTP headers that the server will include in its responses. Default: None
- **disableToHTTPSRedirect** (_bool_)
  - Option to disable redirects from the HTTP server to the HTTPS server. When set to `true`, the server will not automatically redirect HTTP requests to HTTPS. Default: `false`
- **wwwredirect** (_bool_)
  - Option to enable redirects to domain name that begins with "www.". When set to `true`, the server will automatically redirect requests to the "www" subdomain. Default: `false`
- **enableIPSpoofing** (_bool_)
  - Option to enable identifying client’s originating IP address through the X-Forwarded-For header. When set to `true`, the server will use the X-Forwarded-For header to identify the client's IP address. Default: `false`
- **allowDoubleSlashes** (_bool_)
  - Option to allow double slashes in URL sanitizer. When set to `true`, the server will allow double slashes in URLs, which may be useful for certain types of URL rewriting. Default: `false`
- **rewriteMap** (_Array&lt;Object&gt;_)
  - URL rewrite map. This setting specifies an array of URL rewrite rules that the server will apply to incoming requests. Default: None
  - **Sub-properties**:
    - **regex** (_String_)
      - Regular expression for URL rewriting. This setting specifies the regular expression pattern that the server will use to match URLs for rewriting. Default: None
    - **replacement** (_String_)
      - Replacement string for URL rewriting. This setting specifies the replacement string that the server will use to rewrite matched URLs. Default: None
    - **isNotFile** (_bool_)
      - Option to apply the rule only if the path is not a file. When set to `true`, the server will only apply the rewrite rule if the path does not correspond to a file. Default: `false`
    - **isNotDirectory** (_bool_)
      - Option to apply the rule only if the path is not a directory. When set to `true`, the server will only apply the rewrite rule if the path does not correspond to a directory. Default: `false`
    - **allowDoubleSlashes** (_bool_)
      - Option to allow double slashes in the rewritten URL. When set to `true`, the server will allow double slashes in the rewritten URL. Default: `false`
    - **last** (_bool_)
      - Option to stop processing further rules after this one. When set to `true`, the server will stop processing further rewrite rules after this one. Default: `false`
- **enableRewriteLogging** (_bool_)
  - Option to enable logging of URL rewrites. When set to `true`, the server will log all URL rewrites that it performs. Default: `false`
- **wwwroot** (_String_)
  - Webroot from which static files will be served. This setting specifies the root directory from which the server will serve static files. Default: None
- **disableTrailingSlashRedirects** (_bool_)
  - Option to disable redirects if the path points to a directory. When set to `true`, the server will not automatically redirect requests to a trailing slash if the path points to a directory. Default: `false`
- **users** (_Array&lt;Object&gt;_)
  - User list. This setting specifies an array of user objects that the server will use for authentication. It's recommended to use the `ferron-passwd` tool to generate the user objects. Default: None
  - **Sub-properties**:
    - **user** (_String_)
      - Username. This setting specifies the username for a user. Default: None
    - **pass** (_String_)
      - Password hash. This setting specifies the hashed password for a user. Default: None
- **nonStandardCodes** (_Array&lt;Object&gt;_)
  - Non-standard status codes. This setting specifies an array of non-standard HTTP status codes that the server will use for specific responses. Default: None
  - **Sub-properties**:
    - **scode** (_u16_)
      - Status code. This setting specifies the non-standard HTTP status code. If set to `401`, HTTP basic authentication is enabled (users are specified in `users` configuration property). Default: None
    - **url** (_String_)
      - URL to match or redirect to. This setting specifies the URL pattern that the server will use to match requests or the URL to which the server will redirect requests. Default: None
    - **regex** (_String_)
      - Regular expression to match the URL. This setting specifies the regular expression pattern that the server will use to match URLs. Default: None
    - **location** (_String_)
      - Redirect location. This setting specifies the location to which the server will redirect requests. Default: None
    - **realm** (_String_)
      - Realm for basic authentication. This setting specifies the realm that the server will use for basic authentication. Default: None
    - **disableBruteProtection** (_bool_)
      - Option to disable brute force protection. When set to `true`, the server will disable brute force protection for the specified status code. Default: `false`
    - **userList** (_Array&lt;String&gt;_)
      - List of users allowed to access. This setting specifies an array of usernames that are allowed to access the resource associated with the status code. Default: None
    - **users** (_Array&lt;String&gt;_)
      - List of IP addresses allowed to access. This setting specifies an array of IP addresses that are allowed to access the resource associated with the status code. Default: None
- **errorPages** (_Array&lt;Object&gt;_)
  - Custom error pages. This setting specifies an array of custom error pages that the server will use for specific HTTP status codes. Default: None
  - **Sub-properties**:
    - **scode** (_u16_)
      - Status code. This setting specifies the HTTP status code for which the custom error page will be used. Default: None
    - **path** (_String_)
      - Path to the error page. This setting specifies the file path to the custom error page. Default: None
- **enableETag** (_bool_)
  - Option to enable ETag generation. When set to `true`, the server will generate ETag headers for responses, which can be used for caching purposes. Default: `true`
- **enableCompression** (_bool_)
  - Option to enable HTTP compression. When set to `true`, the server will compress responses using gzip or other compression algorithms to reduce bandwidth usage. Default: `true`
- **enableDirectoryListing** (_bool_)
  - Option to enable directory listings. When set to `true`, the server will generate and display a list of files and directories when a directory is requested. Default: `false`
- **proxyTo** (_String_ or _Array&lt;String&gt;_; _rproxy_ module; Project Karpacz 0.2.0 and newer)
  - Base URL, which reverse proxy will send requests to. HTTP and HTTPS URLs are supported. From Project Karpacz 0.4.0, it's also possible to specify an array of base URLs (requests will be randomly distributed). Default: None
- **secureProxyTo** (_String_ or _Array&lt;String&gt;_; _rproxy_ module; Project Karpacz 0.2.0 and newer)
  - Base URL, which reverse proxy will send requests to, if the client is connected via HTTPS. HTTP and HTTPS URLs are supported. From Project Karpacz 0.4.0, it's also possible to specify an array of base URLs (requests will be randomly distributed). Default: None
- **cacheVaryHeaders** (_Array&lt;String&gt;_; _cache_ module; Project Karpacz 0.4.0 and newer)
  - A list of request headers that can vary in a cache. Supplements the “Vary” response header. Default: None
- **cacheIgnoreHeaders** (_Array&lt;String&gt;_; _cache_ module; Project Karpacz 0.4.0 and newer)
  - A list of response headers that will not be stored in a cache. Default: None
- **maximumCacheResponseSize** (_u64_ or `null`; _cache_ module; Project Karpacz 0.4.0 and newer)
  - A maximum response size to be cached in bytes. If `null`, the maximum response size is unlimited theoretically. Default: `null`
- **cgiScriptExtensions** (_Array&lt;String&gt;_; _cgi_ module; Project Karpacz 0.5.0 and newer)
  - CGI script extensions, which will be handled via CGI handler outside the `cgi-bin` directory. Default: None
- **cgiScriptInterpreters** (_Object_; _cgi_ module; Project Karpacz 0.5.0 and newer)
  - CGI script interpreters used by the CGI handler. The object keys represent the extension, for which a specific interpreter is used, while object values can be either an _Array&lt;String&gt;_ representing first arguments of the CGI script (the first argument is the path to the interpreter), or `null` for removing the default interpreter. Default: None, the default interpreterss are set for _.pl_, _.py_, _.sh_, _.ksh_, _.csh_, _.rb_ and _.php_ extensions, and addtionally _.exe_, _.bat_ and _.vbs_ extensions for Windows.
- **scgiTo** (_String_ or _Array&lt;String&gt;_; _scgi_ module; Project Karpacz 0.6.0 and newer)
  - Base URL, which SCGI client will send requests to. TCP (for example `"tcp://localhost:4000/"`) and Unix socket URLs (only on Unix systems; for example `"unix:///run/scgi.sock"`) are supported. Default: `"tcp://localhost:4000/"`
- **scgiPath** (_String_ or _Array&lt;String&gt;_; _scgi_ module; Project Karpacz 0.6.0 and newer)
  - Base URL, which SCGI client will handle the request if the request URL begins with it. If not specified, the SCGI client will be inactive. Default: None
- **fcgiScriptExtensions** (_Array&lt;String&gt;_; _fcgi_ module; Project Karpacz 0.6.0 and newer)
  - FastCGI script extensions, which will be handled via FastCGI handler outside the specified FastCGI path. Default: None
- **fcgiTo** (_String_ or _Array&lt;String&gt;_; _fcgi_ module; Project Karpacz 0.6.0 and newer)
  - Base URL, which FastCGI client will send requests to. TCP (for example `"tcp://localhost:4000/"`) and Unix socket URLs (only on Unix systems; for example `"unix:///run/fcgi.sock"`) are supported. Default: `"tcp://localhost:4000/"`
- **fcgiPath** (_String_ or _Array&lt;String&gt;_; _fcgi_ module; Project Karpacz 0.6.0 and newer)
  - Base URL, which FastCGI client will handle the request if the request URL begins with it. If not specified, the SCGI client will be inactive. Default: None
- **authTo** (_String_; _fauth_ module; Ferron 1.0.0-beta2 and newer)
  - Base URL, which web server will send requests to for forwarded authentication. HTTP and HTTPS URLs are supported. Default: None
- **forwardedAuthCopyHeaders** (_Array&lt;String&gt;_; _fauth_ module; Ferron 1.0.0-beta2 and newer)
  - A list of response headers that will be copied from the forwarded authentication server response to the original request. Default: None
- **enableLoadBalancerHealthCheck** (_u32_; _rproxy_ module; Ferron 1.0.0-beta3 and newer)
  - Option to enable passive health checks for the load balancer. If the connection by the load balancer fails, the load balancer notes the failed connection, and if there are too many of them, the load balancer temporarily marks the backend server as "bad" and will not choose the backend server. Default: `false`
- **loadBalancerHealthCheckMaximumFails** (_u32_; _rproxy_ module; Ferron 1.0.0-beta3 and newer)
  - Maximum reported failed connections, before a backend server is marked as "bad" and not chosen by the load balancer. Default: `3`

## Server configuration includes

Server configuration includes can be specified in the `include` section in the server configuration (at the same level as `global` and `host` would have). The `include` section is a list of glob patterns that match the configuration file names to be merged.

## Example configuration

Below is the example Ferron web server configuration:

```yaml
global:
  port: 8080
  sport: 8443
  secure: true
  enableHTTP2: true
  http2Settings:
    initialWindowSize: 65536
    maxFrameSize: 16384
    maxConcurrentStreams: 100
    maxHeaderListSize: 8192
    enableConnectProtocol: true
  logFilePath: "/var/log/ferron/access.log"
  errorLogFilePath: "/var/log/ferron/error.log"
  cert: "/etc/ssl/certs/server-cert.pem"
  key: "/etc/ssl/private/server-key.pem"
  tlsMinVersion: "TLSv1.2"
  tlsMaxVersion: "TLSv1.3"
  disableNonEncryptedServer: false
  enableOCSPStapling: true
  blocklist:
    - "192.168.1.100"
    - "10.0.0.5"
  enableCompression: true
  enableDirectoryListing: false
  environmentVariables:
    APP_MODE: "production"
    MAX_THREADS: "16"
  loadModules:
    - "rproxy"

hosts:
  - domain: "example.com"
    serverAdministratorEmail: "admin@example.com"
    wwwredirect: true
    customHeaders:
      X-Frame-Options: "DENY"
      X-Content-Type-Options: "nosniff"
    rewriteMap:
      - regex: "^/old-path/(.*)"
        replacement: "/new-path/$1"
        isNotFile: true
        isNotDirectory: true
        last: true
    wwwroot: "/var/www/example"
    errorPages:
      - scode: 404
        path: "/var/www/example/errors/404.html"
      - scode: 500
        path: "/var/www/example/errors/500.html"
    locations:
      - path: "/static"
        wwwroot: "/var/www/static"
        rewriteMap:
        - regex: "^/static(?:[/?#](.*))?"
          replacement: "/$1"
          last: true
  - domain: "api.example.com"
    serverAdministratorEmail: "api-admin@example.com"
    disableToHTTPSRedirect: false
    allowDoubleSlashes: false
    enableETag: true
    users:
      - user: "admin"
        pass: "$2b$10$hashedpassword12345"
    nonStandardCodes:
      - scode: 401
        url: "/restricted.html"
    proxyTo: "http://backend-service:5000"

# # Uncomment to enable configuration file includes
#include:
#  - /etc/ferron.d/**/*.yaml
```
