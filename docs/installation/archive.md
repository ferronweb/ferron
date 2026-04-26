---
title: Manual installation from archive
description: "Install Ferron 3 from a pre-built archive: download, extract, configure, and run on Windows, macOS, or Unix-like systems."
---

Ferron 3 can be installed manually from pre-built binaries from an archive. Archives are provided for Windows, macOS, Linux and FreeBSD.

## Prerequisites

Before installing Ferron, make sure you have:

- A supported operating system: Windows 10+, Windows Server 2016+, macOS 12+, or a modern Linux distribution.
- Internet connectivity to download the archive.
- On Unix-like systems, the `unzip` or `tar` utility available in your `PATH`.

## Downloading the archive

Visit the [Ferron downloads page](/download) and choose the archive that matches your operating system and architecture:

- **Windows**: `.zip` archive (e.g., `ferron-x86_64-pc-windows-msvc.zip`)
- **macOS**: `.tar.gz` archive (e.g., `ferron-aarch64-apple-darwin.tar.gz`)
- **Linux**: `.tar.gz` archive (e.g., `ferron-x86_64-unknown-linux-gnu.tar.gz`)
- **FreeBSD**: `.tar.gz` archive (e.g., `ferron-x86_64-unknown-freebsd.tar.gz`)

## Installation steps

### 1. Extract the archive

- **Windows**:

  Right-click on the downloaded `.zip` file and select **"Extract All..."** to extract the contents.

- **macOS and Linux**:

  Open a terminal, navigate to the directory containing the downloaded `.tar.gz` file, and extract it:

  ```sh
  mkdir ferron
  tar -xzf ferron-*.tar.gz -C ferron
  ```

  This will create a directory containing the Ferron binaries and configuration files.

### 2. Review the extracted contents

After extraction, you should see the following files and directories:

- `ferron` or `ferron.exe` — the main Ferron web server executable.
- `ferron-kdl2ferron` or `ferron-kdl2ferron.exe` — a tool for converting Ferron 2 KDL configurations to Ferron 3 configurations.
- `ferron-passwd` or `ferron-passwd.exe` — a tool for generating hashed passwords for the server's configuration.
- `ferron-precompress` or `ferron-precompress.exe` — a tool for precompressing static files.
- `ferron-serve` or `ferron-serve.exe` — a command for serving static files with Ferron with zero configuration.
- `ferron.conf` — an example configuration file for Ferron.
- `wwwroot/` — the webroot directory containing the default `index.html` file.

### 3. Configure Ferron

Open the `ferron.conf` file in a text editor and modify it to suit your server's requirements. This file includes settings for server ports, logging, modules, and more. Detailed configuration options are available in the [server configuration reference](/docs/v3/configuration/core-directives).

### 4. Run Ferron

- **Windows**:

  Open Command Prompt, navigate to the extracted directory, and run:

  ```cmd
  ferron.exe
  ```

- **macOS**:

  On macOS, you may need to remove the quarantine attribute first:

  ```sh
  xattr -d com.apple.quarantine ferron
  ```

  Then run:

  ```sh
  ./ferron
  ```

- **Linux**:

  Make the binary executable and run it:

  ```sh
  chmod +x ferron
  ./ferron
  ```

### 5. Access the web server

By default, Ferron serves content from the `wwwroot` directory. Open a web browser and navigate to `http://localhost` to verify the server is running.

If you see a **"Ferron is installed successfully!"** message on the page, the web server is installed and running correctly.

## Reloading the configuration

### Unix-like systems (macOS, Linux)

To reload the configuration without restarting the server, send a `SIGHUP` signal to the `ferron` process:

```sh
kill -HUP $(pidof ferron)
```

## Installing as a Windows service

To install Ferron as a Windows service, use the following command in an elevated PowerShell session (run as administrator):

```powershell
path\to\ferron winservice install -c path\to\ferron.conf
```

Replace `path\to` with the actual path to your Ferron installation directory.

## Notes and troubleshooting

- **File permissions on Linux/macOS** — if you encounter permission errors, ensure the `ferron` binary is executable: `chmod +x ferron`.
- **macOS quarantine** — macOS may block the binary with a quarantine warning. Use `xattr -d com.apple.quarantine ferron` to remove it.
- **Firewall settings** — if you cannot access the server from another machine, ensure your firewall allows incoming connections on the configured port (default: 80).
- **Port conflicts** — if port 80 is already in use, you can change the listen port in `ferron.conf` and update your firewall rules accordingly.
