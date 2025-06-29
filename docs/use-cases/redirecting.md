---
title: Redirecting
---

If you want to redirect the entire website to another website, you can use this configuration:

```kdl
// Example configuration with a redirect to another website. Replace "example.org" with your domain name.
example.org {
    status 302 regex="^/.*" location="https://www.example.com" // Replace "www.example.com" with your desired domain. Also, replace 302 with 301 if you want a permanent redirect.
}
```

If you want to redirect the entire website to another website and keep the URL, you can use this configuration:

```kdl
// Example configuration with a redirect to another website. Replace "example.org" with your domain name.
example.org {
    status 302 regex="^/(.*)" location="https://www.example.com/$1" // Replace "www.example.com" with your desired domain. Also, replace 302 with 301 if you want a permanent redirect.
}
```
