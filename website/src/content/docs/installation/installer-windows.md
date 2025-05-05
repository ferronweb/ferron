---
title: Installation via installer (Windows Server)
---

## Installation steps

### 1. Run the installer

To install Ferron web server, run the following command:

```batch
powershell -c "irm https://downloads.ferronweb.org/install.ps1 | iex"
```

You will be prompted to choose the installation type.

### 2. Access the web server

By default, Ferron serves content from the `%SystemDrive%\ferron\wwwroot` directory. Open a web browser and navigate to `http://localhost` to verify that the server is running and serving the default `index.html` file.

## File structure

Ferron installed via the installer for Windows Server has following file structure:

- _%SystemDrive%\ferron\ferron.exe_ - Ferron web server
- _%SystemDrive%\ferron\ferron-passwd.exe_ - Ferron user management tool
- _%SystemDrive%\ferron\log\access.log_ - Ferron access log in Combined Log Format
- _%SystemDrive%\ferron\log\error.log_ - Ferron error log
- _%SystemDrive%\ferron\wwwroot_ - Ferron's web root
- _%SystemDrive%\ferron\ferron.yaml_ - Ferron configuration

## Managing the Ferron service

### Stopping the service

To stop the Ferron service, run:

```sh
net stop ferron
```

### Restarting the service

To restart the service:

```sh
net stop ferron
net start ferron
```
