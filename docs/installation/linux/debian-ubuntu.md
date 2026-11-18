---
title: Installation via package managers (Debian/Ubuntu)
description: "Install Ferron 3 on Debian/Ubuntu using official APT packages: add the repository key, install ferron3, and manage the systemd service."
---

Official packages are available for Debian, Ubuntu, and derivatives.

## Installation steps

### 1. Add the Ferron repository

Run these commands on Debian or Ubuntu. On a derivative, replace `$(lsb_release -cs)` with the closest matching Debian or Ubuntu codename.

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

### 2. Install Ferron

```bash
sudo apt install ferron3
```

> [!tip]
> Keep Ferron up to date by running `sudo apt update && sudo apt upgrade ferron3`.

#### FIPS-certified cryptography variant

A FIPS-certified variant is available as the `ferron3-fips` package. Install it when you must run Ferron in a FIPS-compliant environment:

```bash
sudo apt install ferron3-fips
```

The `ferron3-fips` package conflicts with `ferron3`, so you cannot install both. A FIPS build restricts cryptography to FIPS-approved algorithms:

- OCSP stapling, TLS cipher suites, and key exchange groups are filtered.
- HTTP basic auth password verification accepts only PBKDF2 hashes. Argon2, scrypt, and bcrypt are rejected.

### 3. Access the web server

Ferron serves content from `/var/www/ferron` by default. Open `http://localhost` in a browser to check that the server serves the default `index.html`.

The page shows "Ferron is ready to serve." when the server works.

> [!tip]
> If you cannot reach the server from another machine, check that the firewall allows connections on port 80. If port 80 is in use, change the listen port in `/etc/ferron/ferron.conf` and reload the service.

## File structure

The Debian/Ubuntu package installs these files:

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
