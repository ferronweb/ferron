---
title: Environment overrides
---

Ferron 1.2.0 and newer supports overriding the server configuration via environment variables that begin with `FERRON_`.

## Supported overrides

- `FERRON_PORT` (Ferron 1.2.0 and newer) - corresponds to the _port_ configuration property.
- `FERRON_SPORT` (Ferron 1.2.0 and newer) - corresponds to the _sport_ configuration property.
- `FERRON_HTTP2_INITIAL_WINDOW_SIZE` (Ferron 1.2.0 and newer) - corresponds to the _initialWindowSize_ subproperty of the _http2Settings_ configuration property.
- `FERRON_HTTP2_MAX_FRAME_SIZE` (Ferron 1.2.0 and newer) - corresponds to the _maxFrameSize_ subproperty of the _http2Settings_ configuration property.
- `FERRON_HTTP2_MAX_CONCURRENT_STREAMS` (Ferron 1.2.0 and newer) - corresponds to the _maxConcurrentStreams_ subproperty of the _http2Settings_ configuration property.
- `FERRON_HTTP2_MAX_HEADER_LIST_SIZE` (Ferron 1.2.0 and newer) - corresponds to the _maxHeaderListSize_ subproperty of the _http2Settings_ configuration property.
- `FERRON_HTTP2_ENABLE_CONNECT_PROTOCOL` (Ferron 1.2.0 and newer) - corresponds to the _enableConnectProtocol_ subproperty of the _http2Settings_ configuration property.
- `FERRON_LOG_FILE_PATH` (Ferron 1.2.0 and newer) - corresponds to the _logFilePath_ configuration property.
- `FERRON_ERROR_LOG_FILE_PATH` (Ferron 1.2.0 and newer) - corresponds to the _errorLogFilePath_ configuration property.
- `FERRON_CERT` (Ferron 1.2.0 and newer) - corresponds to the _cert_ configuration property.
- `FERRON_KEY` (Ferron 1.2.0 and newer) - corresponds to the _key_ configuration property.
- `FERRON_TLS_MIN_VERSION` (Ferron 1.2.0 and newer) - corresponds to the _tlsMinVersion_ configuration property.
- `FERRON_TLS_MAX_VERSION` (Ferron 1.2.0 and newer) - corresponds to the _tlsMaxVersion_ configuration property.
- `FERRON_SECURE` (Ferron 1.2.0 and newer) - corresponds to the _secure_ configuration property.
- `FERRON_ENABLE_HTTP2` (Ferron 1.2.0 and newer) - corresponds to the _enableHTTP2_ configuration property.
- `FERRON_ENABLE_HTTP3` (Ferron 1.2.0 and newer) - corresponds to the _enableHTTP3_ configuration property.
- `FERRON_DISABLE_NON_ENCRYPTED_SERVER` (Ferron 1.2.0 and newer) - corresponds to the _disableNonEncryptedServer_ configuration property.
- `FERRON_ENABLE_OCSP_STAPLING` (Ferron 1.2.0 and newer) - corresponds to the _enableOCSPStapling_ configuration property.
- `FERRON_ENABLE_DIRECTORY_LISTING` (Ferron 1.2.0 and newer) - corresponds to the _enableDirectoryListing_ configuration property.
- `FERRON_ENABLE_COMPRESSION` (Ferron 1.2.0 and newer) - corresponds to the _enableCompression_ configuration property.
- `FERRON_LOAD_MODULES` (Ferron 1.2.0 and newer) - corresponds to the _loadModules_ configuration property. The names of the modules to load are comma-separated.
- `FERRON_BLOCKLIST` (Ferron 1.2.0 and newer) - corresponds to the _blocklist_ configuration property. The blocked IP addresses are comma-separated.
- `FERRON_SNI_HOSTS` (Ferron 1.2.0 and newer) - a list of hosts for SNI (Server Name Indication).
- `FERRON_SNI_*_CERT` (Ferron 1.2.0 and newer) - corresponds to the _cert_ subproperty of the host specified in a _sni_ configuration property. In place of `*`, the SNI hostname is specified (where dots are replaced with underscores, and asterisks with `WILDCARD`).
- `FERRON_SNI_*_KEY` (Ferron 1.2.0 and newer) - corresponds to the _key_ subproperty of the host specified in a _sni_ configuration property. In place of `*`, the SNI hostname is specified (where dots are replaced with underscores, and asterisks with `WILDCARD`).
- `FERRON_ENV_VARS` (Ferron 1.2.0 and newer) - a list of environment variables (Server Name Indication).
- `FERRON_ENV_*` (Ferron 1.2.0 and newer) - corresponds to a subproperty of the a _environmentVariables_ configuration property. In place of `*`, the environment variable name is specified.
