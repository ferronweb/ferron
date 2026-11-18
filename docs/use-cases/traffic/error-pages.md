---
title: Error pages
description: "Serve custom error pages for local errors and proxied upstream failures."
---

Ferron can serve custom pages for local errors and, with error interception, for proxy errors.

## Map status codes to a page

```ferron
example.com {
    root /var/www/html

    error_page 404 /var/www/custom/404.html
    error_page 500 502 503 504 /var/www/custom/50x.html
}
```

One directive can map several status codes to the same file.

The path is absolute or relative to the working directory of Ferron. If the file does not exist, Ferron skips the directive and uses its built-in page.

## Include trace information

Set `error_page_placeholders` to replace `{{trace.id}}` and `{{trace.spanid}}` in the error page:

```ferron
example.com {
    error_page 500 /custom/50x.html
    error_page_placeholders
}
```

For example, this `50x.html` shows the trace ID:

```html
<!DOCTYPE html>
<html>
<head><title>Internal Server Error</title></head>
<body>
    <h1>Something went wrong</h1>
    <p>Trace ID: {{trace.id}}</p>
</body>
</html>
```

> [!note]
> Placeholder substitution reads the file into memory. It disables the zerocopy and sendfile optimization for that response. The effect is small for a typical error page.

## Handle upstream failures

Enable `intercept_errors` on the proxy so Ferron can replace backend error responses:

```ferron
app.example.com {
    location / {
        proxy http://127.0.0.1:3000 {
            intercept_errors
        }
    }

    error_page 502 /custom/502.html
    error_page 503 /custom/503.html
    error_page 504 /custom/504.html
}
```

## Return JSON errors

For an API, use `json_errors` to return a JSON body:

```ferron
api.example.com {
    json_errors
}
```

> [!info]
> See [Configuration: static files](/docs/configuration/content/static-files) for `error_page` options and [Configuration: response](/docs/configuration/routing/response) for status rules.
