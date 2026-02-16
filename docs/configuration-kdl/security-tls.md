---
title: "Configuration: security & TLS"
description: "TLS settings, automatic certificate management, and access control directives for KDL configuration."
---

This page covers KDL directives for TLS configuration, certificate automation, and request-security controls in Ferron.

## Global-only directives

### TLS/SSL & security

- `tls_cipher_suite <tls_cipher_suite: string> [<tls_cipher_suite_2: string> ...]`
  - This directive specifies the supported TLS cipher suites. If using the HTTP/3 protocol (which is experimental in Ferron), the `TLS_AES_128_GCM_SHA256` cipher suite needs to be enabled (it's enabled by default), otherwise the HTTP/3 server wouldn’t start at all. This directive can be specified multiple times. Default: default TLS cipher suite for Rustls
- `tls_ecdh_curves <ecdh_curve: string> [<ecdh_curve: string> ...]`
  - This directive specifies the supported TLS ECDH curves. This directive can be specified multiple times. Default: default ECDH curves for Rustls
- `tls_client_certificate [tls_client_certificate: bool|string]`
  - This directive specifies whether the TLS client certificate verification is enabled. If set to `#true`, the client certificate will be verified against the system certificate store. If set to a string, the client certificate will be verified against the certificate authority in the specified path. Default: `tls_client_certificate #false`
- `tls_min_version <tls_min_version: string>`
  - This directive specifies the minimum TLS version (TLSv1.2 or TLSv1.3) that the server will accept. Default: `tls_min_version "TLSv1.2"`
- `tls_max_version <tls_max_version: string>`
  - This directive specifies the maximum TLS version (TLSv1.2 or TLSv1.3) that the server will accept. Default: `tls_max_version "TLSv1.3"`
- `ocsp_stapling [enable_ocsp_stapling: bool]`
  - This directive specifies whether OCSP stapling is enabled. Default: `ocsp_stapling #true`
- `auto_tls_on_demand_ask <auto_tls_on_demand_ask_url: string|null>`
  - This directive specifies the URL to be used for asking whether to the hostname for automatic TLS on demand is allowed. The server will append the `domain` query parameter with the domain name for the certificate to issue as a value to the URL. It's recommended to configure this option when using automatic TLS on demand to prevent abuse. Default: `auto_tls_on_demand_ask #null`
- `auto_tls_on_demand_ask_no_verification [auto_tls_on_demand_ask_no_verification: bool]`
  - This directive specifies whether the server should not verify the TLS certificate of the automatic TLS on demand asking endpoint. Default: `auto_tls_on_demand_ask_no_verification #false`

## Configuration example

```kdl
* {
    tls_cipher_suite "TLS_AES_256_GCM_SHA384" "TLS_AES_128_GCM_SHA256"
    tls_ecdh_curves "secp256r1" "secp384r1"
    tls_client_certificate #false
    ocsp_stapling
    auto_tls_on_demand_ask "https://auth.example.com/check"
    auto_tls_on_demand_ask_no_verification #false
}
```

## Global and virtual host directives

### TLS/SSL & security

- `tls <certificate_path: string> <private_key_path: string>`
  - This directive specifies the path to the TLS certificate and private key. Default: none
- `auto_tls [enable_automatic_tls: bool]`
  - This directive specifies whether automatic TLS is enabled. Default: `auto_tls #true` when port isn't explicitly specified and if the hostname doesn't look like a local address (`127.0.0.1`, `::1`, `localhost`), otherwise `auto_tls #false`
- `auto_tls_contact <auto_tls_contact: string|null>`
  - This directive specifies the email address used to register an ACME account for automatic TLS. Default: `auto_tls_contact #null`
- `auto_tls_cache <auto_tls_cache: string|null>`
  - This directive specifies the directory to store cached ACME data, such as cached account data and certificates. Default: OS-specific directory, for example on GNU/Linux it can be `/home/user/.local/share/ferron-acme` for the "user" user, on macOS it can be `/Users/user/Library/Application Support/ferron-acme` for the "user" user, on Windows it can be `C:\Users\user\AppData\Local\ferron-acme` for the "user" user. On Docker, it would be `/var/lib/ferron-acme`.
- `auto_tls_letsencrypt_production [enable_auto_tls_letsencrypt_production: bool]`
  - This directive specifies whether the production Let's Encrypt ACME endpoint is used. If set as `auto_tls_letsencrypt_production #false`, the staging Let's Encrypt ACME endpoint is used. Default: `auto_tls_letsencrypt_production #true`
- `auto_tls_challenge <acme_challenge_type: string> [provider=<acme_challenge_provider: string>] [...]`
  - This directive specifies the used ACME challenge type. The supported types are `"http-01"` (HTTP-01 ACME challenge), `"tls-alpn-01"` (TLS-ALPN-01 ACME challenge) and `"dns-01"` (DNS-01 ACME challenge). The `provider` prop defines the DNS provider to use for DNS-01 challenges. Additional props can be passed as parameters for the DNS provider, see automatic TLS documentation. Default: `auto_tls_challenge "tls-alpn-01"`
- `auto_tls_directory <auto_tls_directory: string>`
  - This directive specifies the ACME directory URL from which the certificates are obtained. Overrides `auto_tls_letsencrypt_production` directive. Default: none
- `auto_tls_no_verification [auto_tls_no_verification: bool]`
  - This directive specifies whether to disable the certificate verification of the ACME server. Default: `auto_tls_no_verification #false`
- `auto_tls_profile <auto_tls_profile: string|null>`
  - This directive specifies the ACME profile to use for the certificates. Default: `auto_tls_profile #null`
- `auto_tls_on_demand <auto_tls_on_demand: bool>`
  - This directive specifies whether to enable the automatic TLS on demand. The functionality obtains TLS certificates automatically when a website is accessed for the first time. It's recommended to use either HTTP-01 or TLS-ALPN-01 ACME challenges, as DNS-01 ACME challenges might be slower due to DNS propagation delays. It's also recommended to configure the `auto_tls_on_demand_ask` directive alongside this directive. Default: `auto_tls_on_demand #false`
- `auto_tls_eab (<auto_tls_eab_key_id: string> <auto_tls_eab_key_hmac: string>)|<auto_tls_eab_disabled: null>`
  - This directive specifies the EAB key ID and HMAC for the ACME External Account Binding. The HMAC key value is encoded in a URL-safe Base64 encoding. If set as `auto_tls_eab_disabled #null`, the EAB is disabled. Default: `auto_tls_eab_disabled #null`
- `auto_tls_save_data (<auto_tls_save_certificate_path: string> <auto_tls_save_private_key_path: string>)|<auto_tls_save_data_disabled: null>` (Ferron 2.5.0 or newer)
  - This directive specifies the path to save the obtained TLS certificate and private key when using automatic TLS. This can be useful for debugging purposes or for using the obtained TLS certificate and private key with other software. This directive isn't supported when using it alongside automatic TLS on demand. Default: `auto_tls_save_data #null`
- `auto_tls_post_obtain_command <auto_tls_post_obtain_command: string>|<auto_tls_post_obtain_command_disabled: null>` (Ferron 2.5.0 or newer)
  - This directive specifies the command (no arguments are supported though) to be executed after obtaining a TLS certificate when using automatic TLS. The command will be executed with the following environment variables set: `FERRON_ACME_DOMAIN` (the domain name for which the certificate was obtained; comma-separated if multiple domain names), `FERRON_ACME_CERT_PATH` (the path to the obtained TLS certificate), `FERRON_ACME_KEY_PATH` (the path to the obtained private key). This can be useful for running custom scripts after obtaining a TLS certificate, for example for reloading other software that uses the obtained TLS certificate. This directive is effective only when `auto_tls_save_data` directive is effective. Default: `auto_tls_post_obtain_command #null`

## Configuration example

```kdl
example.com {
    auto_tls
    auto_tls_contact "admin@example.com"
    auto_tls_cache "/var/cache/ferron-acme"
    auto_tls_letsencrypt_production
    auto_tls_challenge "tls-alpn-01"
    auto_tls_profile "default"
    auto_tls_on_demand #false
    auto_tls_eab #null
}

manual-tls.example.com {
    tls "/etc/ssl/certs/example.com.crt" "/etc/ssl/private/example.com.key"
}
```

### Security & access control

- `trust_x_forwarded_for [trust_x_forwarded_for: bool]`
  - This directive specifies whether to trust the value of the `X-Forwarded-For` header. It's recommended to configure this directive if behind a reverse proxy. Default: `trust_x_forwarded_for #false`
- `status <status_code: integer> [url=<url: string>|regex=<regex: string>] [location=<location: string>] [realm=<realm: string>] [brute_protection=<enable_brute_protection: bool>] [users=<users: string>] [allowed=<allowed: string>] [not_allowed=<not_allowed: string>] [body=<response_body: string>]`
  - This directive specifies the custom status code. This directive can be specified multiple times. The `url` prop specifies the request path for this status code. The `regex` prop specifies the regular expression (like `^/ferron(?:$|[/#?])`) for the custom status code. The `location` prop specifies the destination for the redirect; it supports placeholders like `{path}` which will be replaced with the request path. The `realm` prop specifies the HTTP basic authentication realm. The `brute_protection` prop specifies whether the brute-force protection is enabled. The `users` prop is a comma-separated list of allowed users for HTTP authentication. The `allowed` prop is a comma-separated list of IP addresses applicable for the status code. The `not_allowed` prop is a comma-separated list of IP addresses not applicable for the status code. The `body` prop specifies the response body to be sent. Default: none
- `user [username: string] [password_hash: string]`
  - This directive specifies an user with a password hash used for the HTTP basic authentication (it can be either Argon2, PBKDF2, or `scrypt` one). It's recommended to use the `ferron-passwd` tool to generate the password hash. This directive can be specified multiple times. Default: none
- `block (<blocked_ip: string> [<blocked_ip: string> ...])|<not_specified: null>`
  - This directive specifies IP addresses and CIDR ranges to be blocked. If set as `block #null`, this directive is ignored. This directive was global-only before Ferron 2.1.0. This directive can be specified multiple times. Default: none
- `allow (<allowed_ip: string> [<allowed_ip: string> ...])|<not_specified: null>`
  - This directive specifies IP addresses and CIDR ranges to be allowed. If set as `allow #null`, this directive is ignored. This directive was global-only before Ferron 2.1.0. This directive can be specified multiple times. Default: none

## Configuration example

```kdl
example.com {
    trust_x_forwarded_for

    // Basic authentication with custom status codes
    status 401 url="/admin" realm="Admin Area" users="admin,moderator"
    status 403 url="/restricted" allowed="192.168.1.0/24" body="Access denied"
    status 301 url="/old-page" location="/new-page"

    // User definitions for authentication (use `ferron-passwd` to generate password hashes)
    user "admin" "$2b$10$hashedpassword12345"
    user "moderator" "$2b$10$anotherhashedpassword"

    // Limit who can access the site
    block "192.168.1.100" "10.0.0.5"
    allow "192.168.1.0/24" "10.0.0.0/8"
}
```
