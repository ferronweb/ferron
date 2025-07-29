---
title: Redirecting
---

If you want to redirect the entire website to another website, you can use this configuration:

```kdl
// Example configuration with a redirect to another website. Replace "example.org" with your domain name.
example.org {
    // In Ferron 2.0.0-beta.14 and earlier, you would use `status 302 regex="^/.*" location="https://www.example.com"` instead.
    status 302 location="https://www.example.com" // Replace "www.example.com" with your desired domain. Also, replace 302 with 301 if you want a permanent redirect.
}
```

If you want to redirect the entire website to another website and keep the URL, you can use this configuration:

```kdl
// Example configuration with a redirect to another website. Replace "example.org" with your domain name.
example.org {
    // In Ferron 2.0.0-beta.14 and earlier, you would use `status 302 regex="^/(.*)" location="https://www.example.com/$1"` instead.
    status 302 location="https://www.example.com{path}" // Replace "www.example.com" with your desired domain. Also, replace 302 with 301 if you want a permanent redirect.
}
```

### Redirecting from URL without "www." to URL with "www."

If you want to redirect all requests from an URL without "www." to URL with "www.", you can use this configuration:

```kdl
// Example configuration with a redirect from URL without "www." to URL with "www.". Replace "example.com" with your domain name.
example.com {
    // In Ferron 2.0.0-beta.14 and earlier, you would use `status 301 regex="^/(.*)" location="https://www.example.com/$1"` instead.
    status 301 location="https://www.example.com{path}"
}

www.example.com {
    // For this example, let's serve static files
    root "/var/www/example"
}
```
