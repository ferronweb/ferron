---
title: Environment overrides
---
Ferron 1.2.0 and newer supports overriding the server configuration via environment variables that begin with `FERRON_`.

## Supported overrides

- `FERRON_PORT` (Ferron 1.2.0 and newer) - corresponds to the *port* configuration property.
- `FERRON_SPORT` (Ferron 1.2.0 and newer) - corresponds to the *sport* configuration property.
- `FERRON_HTTP2_INITIAL_WINDOW_SIZE` (Ferron 1.2.0 and newer) - corresponds to the *initialWindowSize* subproperty of the *http2Settings* configuration property.
- `FERRON_HTTP2_MAX_FRAME_SIZE` (Ferron 1.2.0 and newer) - corresponds to the *maxFrameSize* subproperty of the *http2Settings* configuration property.
- `FERRON_HTTP2_MAX_CONCURRENT_STREAMS` (Ferron 1.2.0 and newer) - corresponds to the *maxConcurrentStreams* subproperty of the *http2Settings* configuration property.
- `FERRON_HTTP2_MAX_HEADER_LIST_SIZE` (Ferron 1.2.0 and newer) - corresponds to the *maxHeaderListSize* subproperty of the *http2Settings* configuration property.
- `FERRON_HTTP2_ENABLE_CONNECT_PROTOCOL` (Ferron 1.2.0 and newer) - corresponds to the *enableConnectProtocol* subproperty of the *http2Settings* configuration property.
- `FERRON_LOG_FILE_PATH` (Ferron 1.2.0 and newer) - corresponds to the *logFilePath* configuration property.
- `FERRON_ERROR_LOG_FILE_PATH` (Ferron 1.2.0 and newer) - corresponds to the *errorLogFilePath* configuration property.
- `FERRON_CERT` (Ferron 1.2.0 and newer) - corresponds to the *cert* configuration property.
- `FERRON_KEY` (Ferron 1.2.0 and newer) - corresponds to the *key* configuration property.
- `FERRON_TLS_MIN_VERSION` (Ferron 1.2.0 and newer) - corresponds to the *tlsMinVersion* configuration property.
- `FERRON_TLS_MAX_VERSION` (Ferron 1.2.0 and newer) - corresponds to the *tlsMaxVersion* configuration property.
- `FERRON_SECURE` (Ferron 1.2.0 and newer) - corresponds to the *secure* configuration property.
- `FERRON_ENABLE_HTTP2` (Ferron 1.2.0 and newer) - corresponds to the *enableHTTP2* configuration property.
- `FERRON_ENABLE_HTTP3` (Ferron 1.2.0 and newer) - corresponds to the *enableHTTP3* configuration property.
- `FERRON_DISABLE_NON_ENCRYPTED_SERVER` (Ferron 1.2.0 and newer) - corresponds to the *disableNonEncryptedServer* configuration property.
- `FERRON_ENABLE_OCSP_STAPLING` (Ferron 1.2.0 and newer) - corresponds to the *enableOCSPStapling* configuration property.
- `FERRON_ENABLE_DIRECTORY_LISTING` (Ferron 1.2.0 and newer) - corresponds to the *enableDirectoryListing* configuration property.
- `FERRON_ENABLE_COMPRESSION` (Ferron 1.2.0 and newer) - corresponds to the *enableCompression* configuration property.
- `FERRON_LOAD_MODULES` (Ferron 1.2.0 and newer) - corresponds to the *loadModules* configuration property. The names of the modules to load are comma-separated.
- `FERRON_BLOCKLIST` (Ferron 1.2.0 and newer) - corresponds to the *blocklist* configuration property. The blocked IP addresses are comma-separated.
- `FERRON_SNI_HOSTS` (Ferron 1.2.0 and newer) - a list of hosts for SNI (Server Name Indication).
- `FERRON_SNI_*_CERT` (Ferron 1.2.0 and newer) - corresponds to the *cert* subproperty of the host specified in a *sni* configuration property. In place of `*`, the SNI hostname is specified (where dots are replaced with underscores, and asterisks with `WILDCARD`).
- `FERRON_SNI_*_KEY` (Ferron 1.2.0 and newer) - corresponds to the *key* subproperty of the host specified in a *sni* configuration property. In place of `*`, the SNI hostname is specified (where dots are replaced with underscores, and asterisks with `WILDCARD`).
- `FERRON_ENV_VARS` (Ferron 1.2.0 and newer) - a list of environment variables (Server Name Indication).
- `FERRON_ENV_*` (Ferron 1.2.0 and newer) - corresponds to a subproperty of the a *environmentVariables* configuration property. In place of `*`, the environment variable name is specified.