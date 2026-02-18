---
title: Error pages
description: "Serve custom error pages in Ferron and improve reverse-proxy failure UX for 5xx upstream issues."
---

Custom error pages make failures clearer for users and reduce confusion during incidents. Ferron can serve custom pages for local errors and (with interception enabled) upstream proxy errors.

## Custom pages for common errors

```kdl
// Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"

    // Optional contact shown on default 500 page.
    server_administrator_email "ops@example.com"

    error_page 404 "/var/www/errors/404.html"
    error_page 500 "/var/www/errors/500.html"
}
```

## Better UX for upstream failures

When reverse proxying, enable error interception so Ferron can serve custom pages for backend errors:

```kdl
// Replace "app.example.com" with your domain name.
app.example.com {
    proxy "http://127.0.0.1:3000/"
    proxy_intercept_errors

    error_page 502 "/var/www/errors/502.html"
    error_page 503 "/var/www/errors/503.html"
    error_page 504 "/var/www/errors/504.html"
}
```

## Notes and troubleshooting

- Without `proxy_intercept_errors`, backend error responses are passed through from the upstream service.
- Keep error-page files lightweight and readable by the Ferron process.
- Use status-specific pages (`502`, `503`, `504`) for better incident messaging.
- For directive details, see [Configuration: routing & URL processing](/docs/configuration/routing-url-processing) and [Configuration: reverse proxying](/docs/configuration/reverse-proxying).
