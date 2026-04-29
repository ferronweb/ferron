---
title: Installation via Linux installer
description: "Install Ferron 3 on Linux using the installer script: run the command, choose your install method, and manage the service."
---

Ferron can be installed on Linux systems using an interactive installer script. The installer detects your distribution, architecture, and C library, then offers you a choice between installing via a package manager (if packages are available) or as a standalone archive.

## Installation steps

### 1. Run the installer

To install Ferron, run the following command:

```bash
sudo bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

You will be prompted to choose the installation type. If packages are available for your distribution, the installer will offer to install Ferron via your package manager. Otherwise, it installs the archive version directly.

### 2. Access the web server

By default, Ferron serves content from the `/var/www/ferron` directory. Open a web browser and navigate to `http://localhost` to check if the server is running and serving the default `index.html` file.

If you see a "Ferron is installed successfully!" message on the page, the web server is installed successfully and is up and running.

## File structure

Ferron installed via the installer for Linux has the following file structure:

- `/usr/sbin/ferron` — Ferron web server
- `/usr/sbin/ferron-kdl2ferron` — Ferron configuration conversion tool
- `/usr/sbin/ferron-passwd` — Ferron user password generation tool
- `/usr/sbin/ferron-precompress` — Ferron static files precompression tool
- `/usr/sbin/ferron-serve` — Ferron zero-configuration static file serving
- `/var/log/ferron/access.log` — Ferron access log in Combined Log Format
- `/var/log/ferron/error.log` — Ferron error log
- `/var/www/ferron` — Ferron's web root
- `/etc/ferron/ferron.conf` — Ferron configuration

## Updating Ferron

To update Ferron to the latest version, run the installer script again:

```bash
sudo bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

The installer detects existing installations and offers to update them.

## Managing the Ferron service

### Stopping the service

To stop the Ferron service, run:

```sh
sudo systemctl stop ferron # For systemd systems
sudo /etc/init.d/ferron stop # For non-systemd systems
```

### Restarting the service

To restart the service:

```sh
sudo systemctl restart ferron # For systemd systems
sudo /etc/init.d/ferron restart # For non-systemd systems
```

### Reloading the configuration

To reload the configuration without restarting the service:

```sh
sudo systemctl reload ferron # For systemd systems
sudo /etc/init.d/ferron reload # For non-systemd systems
```

## Environment variables

The installer supports several environment variables for automation and advanced use cases. Set them before running the installer script.

| Variable | Description | Example |
|----------|-------------|---------|
| `FERRON_ARCHIVE_PATH` | Path to a locally downloaded Ferron archive for offline installation. The installer will validate the archive and skip the download step. | `/tmp/ferron-3.0.0-x86_64-unknown-linux-gnu.tar.gz` |
| `FERRON_VERSION` | Specify a particular Ferron version to install. If not set, the installer fetches the latest stable version from `dl.ferron.sh`. | `3.0.0` |
| `FERRON_INSTALL_METHOD` | Override the detected install method. Valid values: `archive`, `debian`, `rhel`. Useful when the auto-detection fails. | `archive` |
| `FERRON_INSTALL_MODE` | Set to `update` or `uninstall` to skip the interactive mode selection. Only applies when an existing installation is detected. | `update` |
| `FERRON_REMOVE_USER` | During uninstall, set to `yes` to automatically remove the `ferron` system user and group without prompting. | `yes` |
| `NO_COLOR` | Set to any value to disable colored terminal output. Follows the [NO_COLOR standard](https://no-color.org/). | `1` |

### Examples

**Install from a locally downloaded archive:**

```bash
sudo FERRON_ARCHIVE_PATH=/tmp/ferron-3.0.0-x86_64-unknown-linux-gnu.tar.gz \
    bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

**Install a specific version:**

```bash
sudo FERRON_VERSION=3.0.0 \
    bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

**Non-interactive update:**

```bash
sudo FERRON_INSTALL_MODE=update \
    bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

## Notes and troubleshooting

- **Configuration file location** — the default configuration is at `/etc/ferron/ferron.conf`. After editing, reload the service with `sudo systemctl reload ferron` (or the equivalent for your init system).
- **Firewall settings** — if you cannot access the server from another machine, ensure your firewall allows incoming connections on port 80 (or whichever port you configured).
- **Port conflicts** — if port 80 is already in use, change the listen port in `/etc/ferron/ferron.conf` and reload the service.
- **SELinux** — on RHEL/Fedora systems, the installer automatically configures SELinux contexts and booleans for Ferron. If you encounter permission issues, verify that SELinux is properly configured.
- **Non-interactive mode** — the installer can run without user interaction by setting environment variables before executing the script.
