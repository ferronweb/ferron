---
title: Installation via installer (GNU/Linux)
---

## Installation steps
### 1. Run the installer
To install Ferron web server, run the following command:
```bash
sudo bash -c "$(curl -fsSL https://downloads.ferronweb.org/install.sh)"
```

You will be prompted to choose the installation type, and possibly whenever to install packages containing `unzip` and `setcap`.

### 2. Access the web server

By default, Ferron serves content from the `/var/www/ferron` directory. Open a web browser and navigate to `http://localhost` to verify that the server is running and serving the default `index.html` file.

## File structure

Ferron installed via the installer for GNU/Linux has following file structure:

- _/usr/sbin/ferron_ - Ferron web server
- _/usr/sbin/ferron-passwd_ - Ferron user management tool
- _/usr/lib/libferron\_mod\_*.so_ - dynamically linked modules that can be loaded by Ferron
- _/var/log/ferron/access.log_ - Ferron access log in Combined Log Format
- _/var/log/ferron/error.log_ - Ferron error log
- _/var/www/ferron_ - Ferron's web root
- _/etc/ferron.yaml_ - Ferron configuration

## Managing the Ferron service
### Stopping the service
To stop the Ferron service, run:
```sh
sudo /etc/init.d/ferron stop # For non-systemd systems
sudo systemctl stop ferron # For systemd systems
```

### Restarting the service
To restart the service:
```sh
sudo /etc/init.d/ferron restart # For non-systemd systems
sudo systemctl restart ferron # For systemd systems
```