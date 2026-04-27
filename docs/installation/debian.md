---
title: Installation via package managers (Debian/Ubuntu)
description: "Install Ferron 3 on Debian/Ubuntu using official APT packages: add the repository key, install the ferron3 package, and manage the systemd service."
---

Ferron 3 has official packages available for Debian, Ubuntu, and derivatives. Below are the instructions on how to install Ferron 3 on Debian or Ubuntu via a package manager.

## Installation steps

### 1. Add Ferron's repository

To add Ferron's repository, run the following commands (applicable for Debian and Ubuntu, if you're using a derivative, replace `$(lsb_release -cs)` with the closest matching Debian or Ubuntu version codename):

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

### 3. Access the web server

By default, Ferron serves content from the `/var/www/ferron` directory. Open a web browser and navigate to `http://localhost` to check if the server is running and serving the default `index.html` file.

If you see a "Ferron is installed successfully!" message on the page, the web server is installed successfully and is up and running.

## File structure

Ferron 3 installed via the package for Debian/Ubuntu has the following file structure:

- `/usr/sbin/ferron` - Ferron web server
- `/usr/sbin/ferron-kdl2ferron` - Ferron configuration conversion tool
- `/usr/sbin/ferron-passwd` - Ferron user password generation tool
- `/usr/sbin/ferron-precompress` - Ferron static files precompression tool
- `/usr/sbin/ferron-serve` - Ferron zero-configuration static file serving
- `/var/log/ferron/access.log` - Ferron access log in Combined Log Format
- `/var/log/ferron/error.log` - Ferron error log
- `/var/www/ferron` - Ferron's web root
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

## Notes and troubleshooting

- **Configuration file location** — the default configuration is at `/etc/ferron/ferron.conf`. After editing, reload the service with `sudo systemctl reload ferron`.
- **Firewall settings** — if you cannot access the server from another machine, ensure your firewall allows incoming connections on port 80 (or whichever port you configured).
- **Port conflicts** — if port 80 is already in use, change the listen port in `/etc/ferron/ferron.conf` and reload the service.
- **Package updates** — keep Ferron up to date by running `sudo apt update && sudo apt upgrade ferron3`.
