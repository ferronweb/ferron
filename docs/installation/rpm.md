---
title: Installation via package managers (RHEL/Fedora)
---

Ferron has official packages available for Red Hat Enterprise Linux (RHEL), Fedora, and derivatives. Below are the instructions on how to install Ferron on RHEL or Fedora via a package manager.

## Installation steps

### 1. Add Ferron's repository

To add Ferron's repository, run the following commands:

```bash
# Install packages required for adding a new repository
sudo yum install yum-utils

# Add a new RPM package repository
sudo yum-config-manager --add-repo https://rpm.ferron.sh/ferron.repo
```

### 2. Install Ferron

To install Ferron web server, run the following command:

```bash
sudo yum install ferron
```

### 3. Enable and start the service

To enable and start the Ferron service, run the following commands:

```bash
sudo systemctl enable ferron
sudo systemctl start ferron
```

### 4. Access the web server

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
