---
title: Installation via Windows installer
description: "Install Ferron 3 on Windows using the official setup installer: download, run the setup wizard, and manage the Windows service."
---

An official Windows installer supports x64 and ARM64 on Windows and Windows Server.

## Prerequisites

Before installing Ferron, check that you have:

- Windows 10 or later, or Windows Server 2016 or later.
- Administrator privileges to install the Windows service and modify the system PATH.
- A web browser to download the installer.

## Downloading the installer

Download the installer for your architecture from the [Ferron downloads page](/download):

- x64: `ferron-<version>-x86_64-pc-windows-msvc-setup.exe`
- ARM64: `ferron-<version>-aarch64-pc-windows-msvc-setup.exe`

> [!note]
> The x64 installer runs in emulation on Windows ARM devices. On ARM64 hardware, the native ARM64 installer performs better.

## Installation steps

### 1. Run the installer

Double-click the downloaded `.exe` file to start the setup wizard.

> [!tip]
> If Windows SmartScreen shows a warning, click "More info" and then "Run anyway".

### 2. Follow the setup wizard

1. License agreement: review and accept the terms.
2. Installation directory: Ferron installs to `C:\Program Files\Ferron` by default.
3. Setup options: add Ferron to the system PATH and install it as a Windows service.
4. Ready to install: review the choices and click "Install".

### 3. Finish the installation

Click "Finish" to close the wizard.

### 4. Start the web server

1. Open the Services app (`services.msc`).
2. Find and double-click the `Ferron web server` service.
3. Click Start in the Actions pane.

### 5. Access the web server

Ferron serves content from its `wwwroot` directory by default. Open `http://localhost` in a browser to verify the server works.

The page shows "Ferron is ready to serve." when the server works.

> [!tip]
> If you cannot reach the server from another machine, check that Windows Defender Firewall allows connections on port 80. If port 80 is already in use, change the listen port in `C:\ProgramData\Ferron\ferron.conf` and restart the service.

## File structure

The Windows installer installs these files:

- `C:\Program Files\Ferron\ferron.exe`: main Ferron web server executable.
- `C:\Program Files\Ferron\ferron-fmt.exe`: configuration formatter.
- `C:\Program Files\Ferron\ferron-kdl2ferron.exe`: converts Ferron 2 KDL configurations to Ferron 3 configurations.
- `C:\Program Files\Ferron\ferron-passwd.exe`: generates hashed passwords for the server configuration.
- `C:\Program Files\Ferron\ferron-precompress.exe`: precompresses static files.
- `C:\Program Files\Ferron\ferron-serve.exe`: serves static files with no configuration.

## Managing Ferron as a Windows service

Run these commands in an elevated PowerShell session.

Start the service:

```powershell
Start-Service Ferron
```

Stop the service:

```powershell
Stop-Service Ferron
```

Restart the service:

```powershell
Restart-Service Ferron
```

Reload the configuration by restarting the service:

```powershell
Restart-Service Ferron
```

Check the service status:

```powershell
Get-Service Ferron
```

Uninstall the Windows service before you remove Ferron:

```powershell
Stop-Service Ferron
ferron winservice uninstall
```

## Other CLI commands

Ferron also has commands for configuration files. Run them in Command Prompt or PowerShell:

```cmd
ferron validate -c C:\ProgramData\Ferron\ferron.conf
```

```cmd
ferron adapt -c C:\ProgramData\Ferron\ferron.conf
```
