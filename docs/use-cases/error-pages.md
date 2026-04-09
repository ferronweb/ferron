---
title: Error pages
description: "Serve custom error pages in Ferron and improve reverse-proxy failure UX for 5xx upstream issues."
---

Custom error pages make failures clearer for users and reduce confusion during incidents. Ferron can serve custom pages for local errors and (with error interception enabled) upstream proxy errors.

## Custom pages for common errors

```ferron
example.com {
    location / {
        root /var/www/html
    }

    error_page 404 /custom/404.html
    error_page 500 502 503 504 /custom/50x.html
}
```

Multiple status codes can be mapped to the same error page in a single directive.

## Better UX for upstream failures

When reverse proxying, enable error interception so Ferron can serve custom pages for backend errors:

```ferron
app.example.com {
    location / {
        proxy http://127.0.0.1:3000 {
            intercept_errors true
        }
    }

    error_page 502 /custom/502.html
    error_page 503 /custom/503.html
    error_page 504 /custom/504.html
}
```

## Notes and troubleshooting

- Without `intercept_errors true` inside the `proxy` block, backend error responses are passed through from the upstream service as-is.
- The file path is absolute or relative to the current working directory.
- If the specified error page file does not exist, the directive is skipped and the built-in error page is used instead.
- Only applies when an error response is being generated and no custom response has already been set.
- For directive details, see [Configuration: static file serving](/docs/v3/configuration/static-content#error-pages) and [Configuration: reverse proxying](/docs/v3/configuration/reverse-proxying).
