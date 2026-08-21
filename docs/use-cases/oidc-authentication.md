---
title: OIDC authentication
description: "Protect Ferron websites with single sign-on using an OpenID Connect provider like Authelia or Keycloak."
---

Ferron 2.9.0 and newer can act as an OpenID Connect relying party with the `oidc` module: users are authenticated against an OpenID Connect provider (such as Authelia, Keycloak, or any other spec-compliant provider) using the authorization code flow with PKCE, without adding authentication logic to your backend applications.

When an unauthenticated user opens a protected website, Ferron redirects them to the OIDC provider's login page. After a successful login, the user is redirected back to Ferron, which verifies the ID token, stores the identity in an encrypted session cookie, and provides the `Remote-User`, `Remote-Groups`, `Remote-Email`, and `Remote-Name` request headers to backend servers.

Unlike [forward authentication](/docs/use-cases/forward-auth) (the `fauth` module), the `oidc` module doesn't need a per-request round trip to the authentication service; the session is verified by Ferron itself.

## Protect an app with Authelia

Ferron configuration:

```kdl
app.example.com {
    auth_oidc "https://auth.example.com"
    auth_oidc_client_id "ferron"
    auth_oidc_client_secret "{env.FERRON_OIDC_CLIENT_SECRET}"
    auth_oidc_scopes "openid" "profile" "email" "groups"
    auth_oidc_cookie_secret "{env.FERRON_OIDC_COOKIE_SECRET}"
    auth_oidc_logout_path "/.ferron/oidc/logout"

    proxy "http://127.0.0.1:3000/"
}
```

Generate the cookie secret with `openssl rand -base64 32`, and generate the client secret and its hash with `authelia crypto hash generate pbkdf2 --random`.

Authelia client registration (in Authelia's `configuration.yml`):

```yaml
identity_providers:
  oidc:
    clients:
      - client_id: "ferron"
        client_name: "Ferron"
        client_secret: "$pbkdf2-sha512$310000$..." # The hashed client secret
        public: false
        authorization_policy: "one_factor"
        require_pkce: true
        pkce_challenge_method: "S256"
        redirect_uris:
          - "https://app.example.com/.ferron/oidc/callback"
        scopes:
          - "openid"
          - "profile"
          - "email"
          - "groups"
        token_endpoint_auth_method: "client_secret_basic"
```

## Protect an app with Keycloak

Create a confidential client in your Keycloak realm ("Clients" → "Create client", "Client authentication" enabled) with the valid redirect URI `https://app.example.com/.ferron/oidc/callback`, then configure Ferron:

```kdl
app.example.com {
    auth_oidc "https://keycloak.example.com/realms/myrealm"
    auth_oidc_client_id "ferron"
    auth_oidc_client_secret "{env.FERRON_OIDC_CLIENT_SECRET}"
    auth_oidc_cookie_secret "{env.FERRON_OIDC_COOKIE_SECRET}"

    proxy "http://127.0.0.1:3000/"
}
```

To use group-based access control with Keycloak, add a "Group Membership" mapper (with full group paths disabled) that maps groups into a `groups` claim in the ID token.

## Restrict access to specific groups

```kdl
app.example.com {
    auth_oidc "https://auth.example.com"
    auth_oidc_client_id "ferron"
    auth_oidc_client_secret "{env.FERRON_OIDC_CLIENT_SECRET}"
    auth_oidc_scopes "openid" "profile" "email" "groups"
    auth_oidc_cookie_secret "{env.FERRON_OIDC_COOKIE_SECRET}"
    auth_oidc_allowed_groups "admins" "developers"

    proxy "http://127.0.0.1:3000/"
}
```

Users that authenticate successfully but don't belong to any of the allowed groups receive a 403 Forbidden response.

## Protect only selected paths

Apply OIDC authentication only where needed:

```kdl
app.example.com {
    // Public content (no authentication).
    location "/public" {
        root "/var/www/public"
    }

    // Everything else requires login.
    location "/" {
        auth_oidc "https://auth.example.com"
        auth_oidc_client_id "ferron"
        auth_oidc_client_secret "{env.FERRON_OIDC_CLIENT_SECRET}"
        auth_oidc_cookie_secret "{env.FERRON_OIDC_COOKIE_SECRET}"
        proxy "http://127.0.0.1:3000/"
    }
}
```

## Notes

- The `/.ferron/oidc/callback` path (or the path configured with `auth_oidc_redirect_path`) is handled by Ferron itself and isn't passed to backend servers.
- Requests that look like API calls (non-GET/HEAD methods, `Sec-Fetch-Mode: cors`, or `X-Requested-With` header) receive a 401 Unauthorized response instead of a redirect to the login page.
- Always configure `auth_oidc_cookie_secret` in production; without it, a random secret is generated at startup, so sessions don't survive server restarts and can't be shared between multiple server instances.
- The user's session lives in an encrypted cookie; Ferron doesn't store OAuth2 tokens. Refresh tokens and RP-initiated logout at the OIDC provider aren't supported yet.
- Ensure that the clocks of Ferron and the OIDC provider are synchronized (for example with NTP), as ID token validation is time-sensitive.

For the full directive reference, see the [configuration reference](/docs/configuration/security-tls#openid-connect-authentication).
