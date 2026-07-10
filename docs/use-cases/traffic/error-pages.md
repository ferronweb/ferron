---
title: Error pages
description: "Serve custom error pages in Ferron and improve reverse-proxy failure UX for 5xx upstream issues."
---

Custom error pages make failures clearer for users and reduce confusion during incidents. Ferron can serve custom pages for local errors and (with error interception enabled) upstream proxy errors.

## Custom pages for common errors

```ferron
example.com {
    root /var/www/html

    error_page 404 /custom/404.html
    error_page 500 502 503 504 /custom/50x.html
}
```

Multiple status codes can be mapped to the same error page in a single directive.

### Including trace information

Set `error_page_placeholders true` to enable `{{trace.id}}` and `{{trace.spanid}}` placeholders in your error page files. When a request triggers an error, the placeholders are replaced with the request's trace context before the page is served.

```ferron
example.com {
    error_page 500 /custom/50x.html
    error_page_placeholders true
}
```

Example `50x.html`:

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
> Placeholder substitution reads the file into memory, disabling the zerocopy/sendfile optimization for that response. This is negligible for typical error pages.

## Better UX for upstream failures

When reverse proxying, enable error interception so Ferron can serve custom pages for backend errors:

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

> [!note]
> The file path is absolute or relative to the current working directory. If the specified error page file does not exist, the directive is skipped and the built-in error page is used instead.

## JSON error responses

You can serve JSON error responses using the `json_errors` directive (for example for RESTful APIs):

```ferron
api.example.com {
    json_errors
}
```
