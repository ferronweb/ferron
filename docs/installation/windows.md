---
title: Installation via Windows installer
description: "Install Ferron 3 on Windows using the official setup installer: download, run the setup wizard, and manage the Windows service."
---

Ferron 3 provides an official Windows installer for easy setup on Windows and Windows Server. The installer supports both x64 and ARM64 architectures.

## Prerequisites

Before installing Ferron, make sure you have:

- Windows 10+ or Windows Server 2016+.
- Administrator privileges to install the Windows service and modify the system PATH.
- A web browser to download the installer.

## Downloading the installer

Visit the [Ferron downloads page](/download) and download the installer that matches your architecture:

- **x64**: `ferron-<version>-x86_64-pc-windows-msvc-setup.exe`
- **ARM64**: `ferron-<version>-aarch64-pc-windows-msvc-setup.exe`

## Installation steps

### 1. Run the installer

Double-click the downloaded `.exe` installer to launch the setup wizard. If Windows SmartScreen shows a warning, click **"More info"** and then **"Run anyway"**.

### 2. Follow the setup wizard

The installer will guide you through the following steps:

1. **License agreement** — Review the license and accept the terms to continue.
2. **Installation directory** — By default, Ferron is installed to `C:\Program Files\Ferron`. You may change this location if needed.
3. **Setup options** — You can choose to:
   - **Add Ferron to the system PATH** — This lets you run `ferron` from any command prompt or PowerShell session.
   - **Install Ferron as a Windows service** — This makes Ferron start automatically with Windows and run in the background.
4. **Ready to install** — Review your choices and click **"Install"** to begin.

### 3. Finish the installation

Once the installer completes, click **"Finish"** to close the wizard.

### 4. Start the web server

After installing Ferron, start a Windows service by following these steps:

1. Open the Services app (`services.msc`).
2. Find and double-click on the `Ferron web server` service.
3. Click **Start** in the Actions pane to begin.

### 5. Access the web server

By default, Ferron serves content from its `wwwroot` directory. Open a web browser and navigate to `http://localhost` to verify the server is running.

If you see a **"Ferron is installed successfully!"** message on the page, the web server is installed and running correctly.

## File structure

Ferron 3 installed via the Windows installer has the following file structure:

- `C:\Program Files\Ferron\ferron.exe` — the main Ferron web server executable.
- `C:\Program Files\Ferron\ferron-kdl2ferron.exe` — a tool for converting Ferron 2 KDL configurations to Ferron 3 configurations.
- `C:\Program Files\Ferron\ferron-passwd.exe` — a tool for generating hashed passwords for the server's configuration.
- `C:\Program Files\Ferron\ferron-precompress.exe` — a tool for precompressing static files.
- `C:\Program Files\Ferron\ferron-serve.exe` — a command for serving static files with Ferron with zero configuration.
- `C:\Program Files\Ferron\wwwroot\` — the webroot directory containing the default `index.html` file.
- `C:\ProgramData\Ferron\ferron.conf` — the default configuration file (created on first installation).

## Managing Ferron as a Windows service

### Starting the service

To start the Ferron service, run the following in an elevated PowerShell session (run as administrator):

```powershell
Start-Service Ferron
```

### Stopping the service

To stop the Ferron service:

```powershell
Stop-Service Ferron
```

### Restarting the service

To restart the Ferron service:

```powershell
Restart-Service Ferron
```

### Reloading the configuration

To reload the configuration without restarting the service:

```powershell
Restart-Service Ferron
```

### Viewing service status

To check the status of the Ferron service:

```powershell
Get-Service Ferron
```

### Uninstalling the service

To uninstall the Windows service before removing Ferron:

```powershell
Stop-Service Ferron
ferron winservice uninstall
```

## Other CLI commands

Ferron also provides several commands for working with configuration files. Open Command Prompt or PowerShell and run:

```cmd
ferron validate -c C:\ProgramData\Ferron\ferron.conf
```

```cmd
ferron adapt -c C:\ProgramData\Ferron\ferron.conf
```

## Notes and troubleshooting

- **Configuration file location** — the default configuration is at `C:\ProgramData\Ferron\ferron.conf`. After editing, restart the service with `Restart-Service Ferron`.
- **Administrator privileges** — installing as a Windows service and adding to the system PATH require administrator rights. Run PowerShell as administrator when managing the service.
- **Firewall settings** — if you cannot access the server from another machine, ensure Windows Defender Firewall allows incoming connections on port 80 (or whichever port you configured).
- **Port conflicts** — if port 80 is already in use (e.g., by IIS), change the listen port in `C:\ProgramData\Ferron\ferron.conf` and restart the service.
- **PATH updates** — if you chose to add Ferron to the system PATH during installation, you may need to close and reopen your terminal for the changes to take effect.
- **ARM64 support** — the ARM64 installer is compatible with x64 emulated mode on Windows ARM devices, but for best performance use the native ARM64 installer on ARM64 hardware.
