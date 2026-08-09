---
title: Installation via package managers (Debian/Ubuntu)
description: "Install Ferron 3 on Debian/Ubuntu using official APT packages: add the repository key, install ferron3, and manage the systemd service."
---

Ferron 3 has official packages available for Debian, Ubuntu, and derivatives. Below are the instructions on how to install Ferron 3 on Debian or Ubuntu via a package manager.

## Installation steps

### 1. Add the Ferron repository

To add the Ferron repository, run the following commands (applicable for Debian and Ubuntu). If you use a derivative, replace `$(lsb_release -cs)` with the closest matching Debian or Ubuntu version codename:

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

To install Ferron 3, run the following command:

```bash
sudo apt install ferron3
```

> [!tip]
> Keep Ferron up to date by running `sudo apt update && sudo apt upgrade ferron3`.

#### FIPS-certified cryptography variant

A FIPS-certified variant is available as the `ferron3-fips` package. Install it if you must run Ferron in a FIPS-compliant environment:

```bash
sudo apt install ferron3-fips
```

The `ferron3-fips` package conflicts with the standard `ferron3` package, so you cannot install both at the same time. A FIPS build restricts cryptography to FIPS-approved algorithms: OCSP stapling, TLS cipher suites and key exchange groups are filtered, and HTTP basic auth password verification accepts only PBKDF2 hashes (Argon2 and scrypt are rejected).

### 3. Access the web server

By default, Ferron serves content from the `/var/www/ferron` directory. Open a web browser and navigate to `http://localhost` to check if the server works and serves the default `index.html` file.

If you see a "Ferron is installed successfully!" message on the page, the web server works correctly.

> [!tip]
> If you cannot access the server from another machine, make sure your firewall allows incoming connections on port 80. If port 80 is in use, change the listen port in `/etc/ferron/ferron.conf` and reload the service.

## File structure

Ferron 3 installed via the package for Debian/Ubuntu has the following file structure:

- `/usr/sbin/ferron` - Ferron web server
- `/usr/sbin/ferron-fmt` - Ferron configuration formatter
- `/usr/sbin/ferron-kdl2ferron` - Ferron configuration conversion tool
- `/usr/sbin/ferron-passwd` - Ferron user password generation tool
- `/usr/sbin/ferron-precompress` - Ferron static files precompression tool
- `/usr/sbin/ferron-serve` - Ferron zero-configuration static file serving
- `/var/log/ferron/access.log` - Ferron access log in Combined Log Format
- `/var/log/ferron/error.log` - Ferron error log
- `/var/www/ferron` - the Ferron web root
- `/etc/ferron/ferron.conf` - Ferron configuration

## Managing the Ferron service

### Stopping the service

To stop the Ferron service, run:

```sh
sudo systemctl stop ferron
```

### Restarting the service

To restart the service:

```sh
sudo systemctl restart ferron
```

### Reloading the configuration

To reload the configuration without restarting the service:

```sh
sudo systemctl reload ferron
```
