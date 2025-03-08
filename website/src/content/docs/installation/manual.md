---
title: Manual installation
---

## Installation steps

1. **Download the Ferron ZIP archive**

   Obtain the latest Ferron ZIP archive suitable for your operating system from the [Ferron downloads page](/download).

2. **Extract the ZIP archive**:

   - **Unix-like systems**:

     Open a terminal and navigate to the directory containing the downloaded ZIP file. Use the following command to extract the contents:
     ```bash
     unzip ferron.zip
     ```

   - **Windows**:

     Right-click on the ZIP file and select "Extract All..." to unzip the contents.
3. **Review the extracted contents**:

   After extraction, you should see the following files and directories:
   - `ferron` or `ferron.exe` - the main Ferron web server executable.
   - `ferron-passwd` or `ferron-passwd.exe` - a tool for generating user entries with hashed passwords for the server's configuration.
   - `libferron_mod_*.so` or `ferron_mod_*.dll` - dynamically linked modules that can be optionally loaded by Ferron.
   - `ferron.yaml` - an example configuration file for Ferron.
   - `wwwroot/` - the webroot directory containing the default `index.html` file.
4. **Configure Ferron**:

   Modify the `ferron.yaml` configuration file to suit your server's requirements. This file includes settings for server ports, logging, modules, and more. Detailed configuration options are available in the [server configuration properties page](/docs/configuration).
5. **Run Ferron**:

   - **Unix-like systems**:

     In the terminal, navigate to the directory containing the `ferron` executable and run:
     ```bash
     ./ferron
     ```
   - **Windows**:

     Open Command Prompt, navigate to the directory containing `ferron.exe`, and execute:
     ```cmd
     ferron.exe
     ```
6. **Access the web server**:

   By default, Ferron serves content from the `wwwroot` directory. Open a web browser and navigate to `http://localhost` to verify that the server is running and serving the default `index.html` file.

## Additional tools

- **Ferron password tool**:

   The `ferron-passwd` tool assists in generating user entries with hashed passwords for secure configurations. To use it:
   - **Unix-like Systems**:
     ```bash
     ./ferron-passwd someuser
     ```
   - **Windows**:
     ```cmd
     ferron-passwd.exe someuser
     ```

   Follow the on-screen prompts to generate the necessary entries for your configuration file.
- **Modules**:

   Ferron supports dynamically linked modules (`libferron_mod_*.so` or `ferron_mod_*.dll`) to extend its functionality. To load a module, specify the module name in the `loadModules` property of the `ferron.yaml` configuration file.