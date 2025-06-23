---
title: Static file serving
---

Configuring Ferron as a static file server is straightforward - you just need to specify the directory containing your static files in the `root` directive. To configure Ferron as a static file server, you can use the configuration below:

```kdl
// Example global configuration with static file serving
* {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
}
```

## HTTP compression for static files

HTTP compression for static files is enabled by default. To disable it, you can use this configuration:

```kdl
// Example global configuration with static file serving and HTTP compression disabled
* {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    compressed #false
}
```

## Directory listings

Directory listings are disabled by default. To enable them, you can use this configuration:

```kdl
// Example global configuration with static file serving and directory listings enabled
* {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    directory_listing
}
```

## Single-page applications

Single-page applications (SPAs) are also supported by Ferron by adding an URL rewrite rule (if using static file serving only) in addition to the static file serving configuration. You can use this configuration:

```kdl
// Example global configuration with static file serving and URL rewrite rule for SPAs
* {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    rewrite "^/.*" "/" directory=#false file=#false last=#true
}
```
