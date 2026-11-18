---
title: Installation via Linux installer
description: "Install Ferron 3 on Linux using the installer script: run the command, choose your install method, and manage the service."
---

An interactive installer script installs Ferron on Linux. It detects the distribution, architecture, and C library, then offers a package manager install or a standalone archive.

## Installation steps

### 1. Run the installer

```bash
sudo bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

The installer asks for the installation type. When packages exist for the distribution, it offers the package manager install. Otherwise it installs the archive directly.

> [!note]
> On RHEL/Fedora, the installer configures SELinux contexts and booleans for Ferron. If permission errors appear, check the SELinux configuration.

### 2. Access the web server

Ferron serves content from `/var/www/ferron` by default. Open `http://localhost` in a browser to check that the server serves the default `index.html`.

The page shows "Ferron is ready to serve." when the server works.

> [!tip]
> If you cannot reach the server from another machine, check that the firewall allows connections on the configured port, port 80 by default. If port 80 is already in use, change the listen port in `/etc/ferron/ferron.conf` and reload the service.

## File structure

The Linux installer creates these files:

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

## Updating Ferron

Run the installer script again to update Ferron:

```bash
sudo bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

The installer detects an existing installation and offers to update it.

## Managing the Ferron service

Stop the service:

```sh
sudo systemctl stop ferron # For systemd systems
sudo /etc/init.d/ferron stop # For non-systemd systems
```

Restart the service:

```sh
sudo systemctl restart ferron # For systemd systems
sudo /etc/init.d/ferron restart # For non-systemd systems
```

Reload the configuration without a restart:

```sh
sudo systemctl reload ferron # For systemd systems
sudo /etc/init.d/ferron reload # For non-systemd systems
```

## Environment variables

Set these variables before running the installer to automate the install or change its behavior.

| Variable                | Description                                                                                                                                 | Example                                             |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| `FERRON_ARCHIVE_PATH`   | Path to a locally downloaded Ferron archive for offline installation. The installer will validate the archive and skip the download step.   | `/tmp/ferron-3.0.0-x86_64-unknown-linux-gnu.tar.gz` |
| `FERRON_VERSION`        | Specify a particular Ferron version to install. If not set, the installer fetches the latest stable version from `dl.ferron.sh`.            | `3.0.0`                                             |
| `FERRON_INSTALL_METHOD` | Override the detected install method. Valid values: `archive`, `debian`, `rhel`. Useful when the auto-detection fails.                      | `archive`                                           |
| `FERRON_INSTALL_MODE`   | Set to `update` or `uninstall` to skip the interactive mode selection. It only applies when the installer detects an existing installation. | `update`                                            |
| `FERRON_REMOVE_USER`    | During uninstall, set to `yes` to automatically remove the `ferron` system user and group without prompting.                                | `yes`                                               |
| `NO_COLOR`              | Set to any value to disable colored terminal output. Follows the [NO_COLOR standard](https://no-color.org/).                                | `1`                                                 |

### Examples

Install from a locally downloaded archive:

```bash
sudo FERRON_ARCHIVE_PATH=/tmp/ferron-3.0.0-x86_64-unknown-linux-gnu.tar.gz \
    bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

Install a specific version:

```bash
sudo FERRON_VERSION=3.0.0 \
    bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

Update without prompts:

```bash
sudo FERRON_INSTALL_MODE=update \
    bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```
