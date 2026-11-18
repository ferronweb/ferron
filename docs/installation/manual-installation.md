---
title: Manual installation from archive
description: "Install Ferron 3 from a pre-built archive: download, extract, configure, and run on Windows, macOS, or Unix-like systems."
---

Ferron also ships as pre-built archives for Windows, macOS, Linux, and FreeBSD.

## Prerequisites

- A supported operating system: Windows 10 or later, Windows Server 2016 or later, macOS 12 or later, or a modern Linux distribution.
- Internet access to download the archive.
- On Unix-like systems, `unzip` or `tar` in `PATH`.

## Downloading the archive

Choose the archive for the operating system and architecture on the [Ferron downloads page](/download):

- Windows: `.zip` archive, for example `ferron-3.0.0-x86_64-pc-windows-msvc.zip`
- macOS: `.tar.gz` archive, for example `ferron-3.0.0-aarch64-apple-darwin.tar.gz`
- Linux: `.tar.gz` archive, for example `ferron-3.0.0-x86_64-unknown-linux-gnu.tar.gz`
- FreeBSD: `.tar.gz` archive, for example `ferron-3.0.0-x86_64-unknown-freebsd.tar.gz`

> [!note]
> FIPS-certified archives carry a `+fips` suffix, for example `ferron+fips-3.0.0-x86_64-unknown-linux-gnu.tar.gz`. A FIPS archive restricts cryptography to FIPS-approved algorithms:
>
> - OCSP stapling, TLS cipher suites, and key exchange groups are filtered.
> - HTTP basic auth password verification accepts only PBKDF2 hashes. Argon2, scrypt, and bcrypt are rejected.
>
> Use a FIPS archive when you must run Ferron in a FIPS-compliant environment.

Every release also publishes a software bill of materials (SBOM) archive. See [Software bill of materials](#software-bill-of-materials-sbom).

## Installation steps

### 1. Extract the archive

On Windows, right-click the downloaded `.zip` file and select "Extract All...".

On macOS and Linux, open a terminal in the directory with the `.tar.gz` file and extract it:

```sh
mkdir ferron
tar -xzf ferron-*.tar.gz -C ferron
```

The directory then contains the Ferron binaries and configuration files.

### 2. Verify the archive (Linux, FreeBSD)

```sh
# 1. Import the signing key (if not already imported)
wget https://dl.ferron.sh/signing.pgp
gpg --import signing.pgp

# 2. Download the .asc file for the downloaded .tar.gz archive
# This example assumes ferron-3.0.0-x86_64-unknown-linux-gnu.tar.gz
#wget https://dl.ferron.sh/3.0.0/ferron-3.0.0-x86_64-unknown-linux-gnu.tar.gz.asc

# 3. Verify the detached signature
gpg --verify ferron-*.tar.gz.asc
```

### 3. Review the extracted contents

- `ferron` or `ferron.exe`: main Ferron web server executable.
- `ferron-fmt` or `ferron-fmt.exe`: configuration formatter.
- `ferron-kdl2ferron` or `ferron-kdl2ferron.exe`: converts Ferron 2 KDL configurations to Ferron 3 configurations.
- `ferron-passwd` or `ferron-passwd.exe`: generates hashed passwords for the server configuration.
- `ferron-precompress` or `ferron-precompress.exe`: precompresses static files.
- `ferron-serve` or `ferron-serve.exe`: serves static files with no configuration.
- `ferron.conf`: example configuration file.
- `wwwroot/`: web root with the default `index.html` file.

### 4. Configure Ferron

Open `ferron.conf` in a text editor and adjust it to the server. The file covers ports, logging, modules, and more. For the available directives, see the [server configuration reference](/docs/configuration/server/core-directives).

### 5. Run Ferron

On Windows, open Command Prompt in the extracted directory:

```cmd
ferron.exe
```

On macOS:

```sh
./ferron
```

> [!tip]
> On macOS, remove the quarantine attribute first if the system blocks the binary:
>
> ```sh
> xattr -d com.apple.quarantine ferron
> ```

On Linux, make the binary executable and run it:

```sh
chmod +x ferron
./ferron
```

### 6. Access the web server

Ferron serves content from `wwwroot` by default. Open `http://localhost` in a browser to verify the server works.

The page shows "Ferron is ready to serve." when the server works.

> [!tip]
> If you cannot reach the server from another machine, check that the firewall allows connections on the configured port, port 80 by default. If port 80 is in use, change the listen port in `ferron.conf` and update the firewall rules.

## Software bill of materials (SBOM)

Ferron publishes a software bill of materials (SBOM) for every release. An SBOM is a machine-readable inventory of the third-party components in a software product. Ferron builds it from the Rust crates compiled into the binaries.

### Download the SBOM archive

An SBOM archive has the same base name as the matching binary archive, with a `-sbom` suffix. The format follows the binary archive: `.tar.gz` for Unix-like systems and `.zip` for Windows.

Examples:

- `ferron-3.0.0-x86_64-unknown-linux-gnu-sbom.tar.gz`
- `ferron-3.0.0-x86_64-pc-windows-msvc-sbom.zip`
- `ferron+fips-3.0.0-x86_64-unknown-linux-gnu-sbom.tar.gz`

GitHub releases attach the SBOM archive next to the binary archive.

### Contents of the SBOM archive

Each archive contains one SBOM document per Ferron binary, in two formats:

- `.cdx.json`: CycloneDX in JSON format.
- `.cdx.xml`: CycloneDX in XML format.

Each document lists the third-party Rust crates in the binary with their versions and package URLs (purl). License information appears when the crate declares it.

### Use the SBOM

Software composition analysis tools such as Grype, Trivy, and Dependency-Track can read the SBOM. They match components against vulnerability databases and license policies. Run the scan before you deploy a release.

## Reload the configuration (Unix)

Send `SIGHUP` to the `ferron` process to reload the configuration without a restart:

```sh
kill -HUP $(pidof ferron)
```

## Install as a Windows service

Run this in an elevated PowerShell session:

```powershell
path\to\ferron winservice install -c path\to\ferron.conf
```

Replace `path\to` with the path to the Ferron installation directory.

## Run as a daemon (Unix)

Run Ferron as a background daemon with a PID file:

```sh
./ferron daemon -c ferron.conf --pid-file /var/run/ferron.pid
```

Reload the daemon through its PID file:

```sh
kill -HUP $(cat /var/run/ferron.pid)
```

## Other CLI commands

```sh
./ferron validate -c ferron.conf   # validate configuration without starting
./ferron adapt -c ferron.conf      # output configuration as JSON
```
