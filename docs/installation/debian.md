---
title: Installation via package managers (Debian/Ubuntu)
---

Ferron has official packages available for Debian, Ubuntu, and derivatives. Below are the instructions on how to install Ferron on Debian or Ubuntu via a package manager.

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

To install Ferron web server, run the following command:

```bash
sudo apt install ferron
```

### 3. Access the web server

By default, Ferron serves content from the `/var/www/ferron` directory. Open a web browser and navigate to `http://localhost` to check if the server is running and serving the default `index.html` file.

If you see a "Ferron is installed successfully!" message on the page, the web server is installed successfully and is up and running.

## File structure

Ferron installed via the installer for GNU/Linux has following file structure:

- _/usr/sbin/ferron_ - Ferron web server
- _/usr/sbin/ferron-passwd_ - Ferron user password generation tool
- _/usr/sbin/ferron-precompress_ - Ferron static files precompression tool
- _/usr/sbin/ferron-yaml2kdl_ - Ferron configuration conversion tool
- _/var/log/ferron/access.log_ - Ferron access log in Combined Log Format
- _/var/log/ferron/error.log_ - Ferron error log
- _/var/www/ferron_ - Ferron's web root
- _/etc/ferron.kdl_ - Ferron configuration

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
