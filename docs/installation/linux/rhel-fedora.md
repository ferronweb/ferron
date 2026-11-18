---
title: Installation via package managers (RHEL/Fedora)
description: "Install Ferron 3 on RHEL/Fedora/openSUSE using official RPM packages: add the yum repo, install the ferron3 package, and manage the systemd service."
---

Official packages are available for Red Hat Enterprise Linux (RHEL), Fedora, openSUSE, and derivatives.

## Installation steps

### 1. Add the Ferron repository

On RHEL or Fedora:

```bash
# Install packages required for adding a new repository
sudo yum install yum-utils

# Add a new RPM package repository
sudo yum-config-manager --add-repo https://rpm.ferron.sh/ferron.repo
```

On openSUSE:

```bash
sudo zypper addrepo https://rpm.ferron.sh/ferron.repo
```

### 2. Install Ferron

On RHEL or Fedora:

```bash
sudo yum install ferron3
```

On openSUSE:

```bash
sudo zypper install ferron3
```

> [!note]
> Keep Ferron up to date with `sudo yum update ferron3` or `sudo zypper update ferron3`.

#### FIPS-certified cryptography variant

A FIPS-certified variant is available as the `ferron3-fips` package. Install it when you must run Ferron in a FIPS-compliant environment:

```bash
sudo yum install ferron3-fips # RHEL/Fedora
sudo zypper install ferron3-fips # openSUSE
```

The `ferron3-fips` package conflicts with `ferron3`, so you cannot install both. A FIPS build restricts cryptography to FIPS-approved algorithms:

- OCSP stapling, TLS cipher suites, and key exchange groups are filtered.
- HTTP basic auth password verification accepts only PBKDF2 hashes. Argon2, scrypt, and bcrypt are rejected.

### 3. Enable and start the service

```bash
sudo systemctl enable ferron
sudo systemctl start ferron
```

### 4. Access the web server

Ferron serves content from `/var/www/ferron` by default. Open `http://localhost` in a browser to check that the server serves the default `index.html`.

The page shows "Ferron is ready to serve." when the installation works.

> [!tip]
> If you cannot reach the server from another machine, check that the firewall allows connections on port 80, or on the port you configured. If port 80 is already in use, change the listen port in `/etc/ferron/ferron.conf` and reload the service.

## File structure

The RHEL/Fedora package installs these files:

- `/usr/sbin/ferron`: Ferron web server
- `/usr/sbin/ferron-fmt`: configuration formatter
- `/usr/sbin/ferron-kdl2ferron`: configuration conversion tool
- `/usr/sbin/ferron-passwd`: user password generation tool
- `/usr/sbin/ferron-precompress`: static file precompression tool
- `/usr/sbin/ferron-serve`: zero-configuration static file serving
- `/var/log/ferron/access.log`: access log in Combined Log Format
- `/var/log/ferron/error.log`: error log
- `/var/www/ferron`: web root
- `/etc/ferron/ferron.conf`: configuration

## Managing the Ferron service

Stop the service:

```sh
sudo systemctl stop ferron
```

Restart the service:

```sh
sudo systemctl restart ferron
```

Reload the configuration without a restart:

```sh
sudo systemctl reload ferron
```
