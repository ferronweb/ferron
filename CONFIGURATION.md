# Server configuration properties

Project Karpacz can be configured in the `project-karpacz.yaml` file. Below is the description of configuration properties for this server.

## Global-only configuration properties

- **port** (*u16*)
   - HTTP port or address-port combination for the server to listen. This is the primary port on which the server will accept incoming HTTP connections. Default: None (must be specified)

- **sport** (*u16*)
   - HTTPS port or address-port combination for the server to listen. This is the primary port on which the server will accept incoming HTTPS connections. Default: None (must be specified)

- **secure** (*bool*)
   - Option to enable HTTPS. When set to `true`, the server will use HTTPS for secure communication. Default: `false`

- **http2Settings** (*Object*)
   - HTTP/2 settings. This object contains various settings related to HTTP/2 protocol configuration. Default: None
   - **Sub-properties**:
     - **initialWindowSize** (*u32*)
       - Initial window size for HTTP/2. This setting controls the initial flow control window size for HTTP/2 connections. Default: None
     - **maxFrameSize** (*u32*)
       - Maximum frame size for HTTP/2. This setting determines the largest frame payload that the server will accept. Default: None
     - **maxConcurrentStreams** (*u32*)
       - Maximum number of concurrent streams for HTTP/2. This setting limits the number of concurrent streams that can be open at any given time. Default: None
     - **maxHeaderListSize** (*u32*)
       - Maximum header list size for HTTP/2. This setting controls the maximum size of the header list that the server will accept. Default: None
     - **enableConnectProtocol** (*bool*)
       - Enable the HTTP/2 CONNECT protocol. When set to `true`, the server will support the CONNECT method for tunneling TCP connections. Default: `false`

- **logFilePath** (*String*)
   - Path to the log file. This setting specifies the file path where the server will write its logs. Default: None

- **errorLogFilePath** (*String*)
   - Path to the error log file. This setting specifies the file path where the server will write its error logs. Default: None

- **enableHTTP2** (*bool*)
   - Option to enable HTTP/2. When set to `true`, the server will support the HTTP/2 protocol. Default: `false`

- **cert** (*String*)
   - Path to the TLS certificate. This setting specifies the file path to the TLS certificate used for HTTPS connections. Default: None

- **key** (*String*)
   - Path to the private key. This setting specifies the file path to the private key associated with the TLS certificate. Default: None

- **sni** (*Object*)
   - SNI certificate and key data. This object contains the certificate and key data for Server Name Indication (SNI), which allows multiple SSL certificates to be used on the same IP address. Default: None
   - **Sub-properties**:
     - **cert** (*String*)
       - Path to the SNI certificate. This setting specifies the file path to the SNI certificate. Default: None
     - **key** (*String*)
       - Path to the SNI private key. This setting specifies the file path to the private key associated with the SNI certificate. Default: None

- **loadModules** (*Array&lt;String&gt;*)
   - Modules to load. This setting specifies an array of modules that the server should load at startup. Default: None

- **useClientCertificate** (*bool*)
   - Option to require client to provide its certificate. When set to `true`, the server will require clients to present a valid certificate for authentication. Default: `false`

- **cipherSuite** (*Array&lt;String&gt;*)
   - A list of cipher suites. This setting specifies an array of cipher suites that the server will support for encrypted connections. Default: None

- **ecdhCurve** (*Array&lt;String&gt;*)
   - A list of ECDH curves. This setting specifies an array of elliptic curves that the server will support for ECDH key exchanges. Default: None

- **tlsMinVersion** (*String*)
   - Minimum TLS version (TLSv1.2 or TLSv1.3). This setting specifies the minimum version of TLS that the server will accept. Default: `"TLSv1.2"`

- **tlsMaxVersion** (*String*)
   - Maximum TLS version (TLSv1.2 or TLSv1.3). This setting specifies the maximum version of TLS that the server will accept. Default: `"TLSv1.3"`

- **disableNonEncryptedServer** (*bool*)
   - Option to disable the HTTP server if the HTTPS server is running. When set to `true`, the server will only accept HTTPS connections and will disable the HTTP server. Default: `false`

- **blocklist** (*Array&lt;String&gt;*)
   - IP block list. This setting specifies an array of IP addresses that the server will block from accessing its services. Default: None

- **enableOCSPStapling** (*bool*)
   - Option to enable OCSP stapling. When set to `true`, the server will use OCSP stapling to provide certificate revocation status to clients. Default: `false`

- **environmentVariables** (*Object*)
   - Environment variables. This object contains environment variables that the server will use during operation. Default: None

## Global & host configuration properties

- **domain** (*String*)
   - The domain name of a host. This setting specifies the domain name associated with the host. Default: None

- **ip** (*String*)
   - The IP address of a host. This setting specifies the IP address associated with the host. Default: None

- **serverAdministratorEmail** (*String*)
   - Server administrator's email address. This setting specifies the email address of the server administrator, which may be used for contact purposes. Default: None

- **customHeaders** (*Object*)
   - Custom HTTP headers. This object contains custom HTTP headers that the server will include in its responses. Default: None

- **disableToHTTPSRedirect** (*bool*)
   - Option to disable redirects from the HTTP server to the HTTPS server. When set to `true`, the server will not automatically redirect HTTP requests to HTTPS. Default: `false`

- **wwwredirect** (*bool*)
   - Option to enable redirects to domain name that begins with "www.". When set to `true`, the server will automatically redirect requests to the "www" subdomain. Default: `false`

- **enableIPSpoofing** (*bool*)
   - Option to enable identifying client’s originating IP address through the X-Forwarded-For header. When set to `true`, the server will use the X-Forwarded-For header to identify the client's IP address. Default: `false`

- **allowDoubleSlashes** (*bool*)
   - Option to allow double slashes in URL sanitizer. When set to `true`, the server will allow double slashes in URLs, which may be useful for certain types of URL rewriting. Default: `false`

- **rewriteMap** (*Array&lt;Object&gt;*)
   - URL rewrite map. This setting specifies an array of URL rewrite rules that the server will apply to incoming requests. Default: None
   - **Sub-properties**:
     - **regex** (*String*)
       - Regular expression for URL rewriting. This setting specifies the regular expression pattern that the server will use to match URLs for rewriting. Default: None
     - **replacement** (*String*)
       - Replacement string for URL rewriting. This setting specifies the replacement string that the server will use to rewrite matched URLs. Default: None
     - **isNotFile** (*bool*)
       - Option to apply the rule only if the path is not a file. When set to `true`, the server will only apply the rewrite rule if the path does not correspond to a file. Default: `false`
     - **isNotDirectory** (*bool*)
       - Option to apply the rule only if the path is not a directory. When set to `true`, the server will only apply the rewrite rule if the path does not correspond to a directory. Default: `false`
     - **allowDoubleSlashes** (*bool*)
       - Option to allow double slashes in the rewritten URL. When set to `true`, the server will allow double slashes in the rewritten URL. Default: `false`
     - **last** (*bool*)
       - Option to stop processing further rules after this one. When set to `true`, the server will stop processing further rewrite rules after this one. Default: `false`

- **enableRewriteLogging** (*bool*)
   - Option to enable logging of URL rewrites. When set to `true`, the server will log all URL rewrites that it performs. Default: `false`

- **wwwroot** (*String*)
   - Webroot from which static files will be served. This setting specifies the root directory from which the server will serve static files. Default: None

- **disableTrailingSlashRedirects** (*bool*)
   - Option to disable redirects if the path points to a directory. When set to `true`, the server will not automatically redirect requests to a trailing slash if the path points to a directory. Default: `false`

- **users** (*Array&lt;Object&gt;*)
   - User list. This setting specifies an array of user objects that the server will use for authentication. Default: None
   - **Sub-properties**:
     - **user** (*String*)
       - Username. This setting specifies the username for a user. Default: None
     - **pass** (*String*)
       - Password hash. This setting specifies the hashed password for a user. Default: None

- **nonStandardCodes** (*Array&lt;Object&gt;*)
   - Non-standard status codes. This setting specifies an array of non-standard HTTP status codes that the server will use for specific responses. Default: None
   - **Sub-properties**:
     - **scode** (*u16*)
       - Status code. This setting specifies the non-standard HTTP status code. Default: None
     - **url** (*String*)
       - URL to match or redirect to. This setting specifies the URL pattern that the server will use to match requests or the URL to which the server will redirect requests. Default: None
     - **regex** (*String*)
       - Regular expression to match the URL. This setting specifies the regular expression pattern that the server will use to match URLs. Default: None
     - **location** (*String*)
       - Redirect location. This setting specifies the location to which the server will redirect requests. Default: None
     - **realm** (*String*)
       - Realm for basic authentication. This setting specifies the realm that the server will use for basic authentication. Default: None
     - **disableBruteProtection** (*bool*)
       - Option to disable brute force protection. When set to `true`, the server will disable brute force protection for the specified status code. Default: `false`
     - **userList** (*Array&lt;String&gt;*)
       - List of users allowed to access. This setting specifies an array of usernames that are allowed to access the resource associated with the status code. Default: None
     - **users** (*Array&lt;String&gt;*)
       - List of IP addresses allowed to access. This setting specifies an array of IP addresses that are allowed to access the resource associated with the status code. Default: None

- **errorPages** (*Array&lt;Object&gt;*)
   - Custom error pages. This setting specifies an array of custom error pages that the server will use for specific HTTP status codes. Default: None
   - **Sub-properties**:
     - **scode** (*u16*)
       - Status code. This setting specifies the HTTP status code for which the custom error page will be used. Default: None
     - **path** (*String*)
       - Path to the error page. This setting specifies the file path to the custom error page. Default: None

- **enableETag** (*bool*)
   - Option to enable ETag generation. When set to `true`, the server will generate ETag headers for responses, which can be used for caching purposes. Default: `true`

- **enableCompression** (*bool*)
   - Option to enable HTTP compression. When set to `true`, the server will compress responses using gzip or other compression algorithms to reduce bandwidth usage. Default: `true`

- **enableDirectoryListing** (*bool*)
   - Option to enable directory listings. When set to `true`, the server will generate and display a list of files and directories when a directory is requested. Default: `false`

- **proxyTo** (*String*; *rproxy* module)
   - Base URL, which reverse proxy will send requests to. HTTP and HTTPS URLs are supported. Default: None

- **secureProxyTo** (*String*; *rproxy* module)
   - Base URL, which reverse proxy will send requests to, if the client is connected via HTTPS. HTTP and HTTPS URLs are supported. Default: None