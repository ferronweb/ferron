---
title: "Ferron 2.0.0-beta.18 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.18. This release brings new features, improvements and fixes.
date: 2025-10-04 07:53:00
cover: ./covers/ferron-2-0-0-beta-18-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.18, with new features, improvements and fixes.

## Key improvements and fixes

### Default ACME cache directory path for Docker

Added a default ACME cache directory path for Docker images. A temporary container for ACME cache owner setting is no longer needed when using the default ACME cache directory path.

### Precompressed static files

Added support for serving precompressed static files, which reduces the load on the server by serving compressed files directly to the client. This feature can be particularly useful for serving static assets like images, CSS, and JavaScript files.

You can now use this configuration to enable serving precompressed static files:

Ferron supports serving precompressed static files. To enable this feature, you can use this configuration:

```kdl
// Example configuration with static file serving and precompressed files enabled. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    precompressed
}
```

### Flipped access and error log filenames fix for Docker

Fixed flipped access and error log filenames in the default server configuration for Docker images. This ensures that the logs are correctly named and organized.

### Built-in certificate store fallback

Added a built-in certificate store fallback mechanism to handle cases where the native certificate store is not available or accessible. This ensures that Ferron can still function even in the absence of a certificate store.

## Docker image tag update

Ferron used to provide these tags for the Ferron 2 image:

- `2` - Based on Distroless, dynamically-linked binaries (GNU libc required)
- `2-alpine` - Based on Chainguard, statically-linked binaries

Ferron now provides the following tags for the Ferron 2 image:

- `2` - Based on Distroless, statically-linked binaries
- `2-alpine` - Based on Alpine Linux, statically-linked binaries
- `2-debian` - Based on Debian GNU/Linux, dynamically-linked binaries (GNU libc required)

We have added Docker image tags that contain a package manager and a shell, allowing for more flexibility and customization. The `2` tags is still available for those preferring a minimal image without a package manager or shell, for better security.

## A new Debian repository

Ferron now provides a new Debian repository that allows users to easily install and update Ferron on Debian, Ubuntu, and derivatives. This repository includes the latest version of Ferron and is regularly updated to ensure that users have access to the most recent features and bug fixes.

You can add the repository to your system like this:

```bash
# Install packages required for adding a new repository
sudo apt install curl gnupg2 ca-certificates lsb-release debian-archive-keyring

# Add the public PGP key
curl https://deb.ferron.sh/signing.pgp | gpg --dearmor | sudo tee /usr/share/keyrings/ferron-keyring.gpg >/dev/null

# Add a new Debian package repository
echo "deb [signed-by=/usr/share/keyrings/ferron-keyring.gpg] https://deb.ferron.sh $(lsb_release -cs) main" | sudo tee /etc/apt/sources.list.d/ferron.list

# Fetch the package lists
sudo apt update
```

And install Ferron using the following command:

```bash
sudo apt install ferron
```

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
