#Requires -RunAsAdministrator

function ExitWithCode
{
    param
    (
        $exitcode
    )

    $host.SetShouldExit($exitcode)
    exit
}

# Print the splash
Write-Host '*******************************************'
Write-Host '**Ferron 2.x installer for Windows Server**'
Write-Host '*******************************************'
Write-Host ''

# Check the CPU architecture
$processor = Get-WmiObject -Class Win32_Processor
$architectureInt = $processor.AddressWidth

if ($processor.Architecture -eq 5)
{
    $architecture = "arm64"
    $triple = "aarch64-pc-windows-msvc"
} elseif ($architectureInt -eq 64)
{
    $architecture = "x64"
    $triple = "x86_64-pc-windows-msvc"
} elseif ($architectureInt -eq 32)
{
    $architecture = "x86"
    $triple = "i686-pc-windows-msvc"
} else
{
    Write-Host "Invalid CPU architecture detected."
    ExitWithCode -exitcode 1
}

# Select Ferron installation type
Write-Host 'Select your Ferron installation type. Valid Ferron installation types:'
Write-Host '0 - Latest stable version'
Write-Host '1 - Install and update manually'
$ITP = Read-Host 'Your Ferron installation type'

switch ($ITP)
{
    0
    { $installType = 'stable'
    }
    1
    { $installType = 'manual'
    }
    default
    { Write-Host 'Invalid Ferron installation type!'; ExitWithCode -exitcode 1
    }
}

# Create a Ferron installation directory
if (!(Test-Path -Path "$env:SYSTEMDRIVE\ferron"))
{
    New-Item -Path "$env:SYSTEMDRIVE\" -Name "ferron" -ItemType "directory" | Out-Null
}

# Create subdirectories
if (!(Test-Path -Path "$env:SYSTEMDRIVE\ferron\logs"))
{
    New-Item -Path "$env:SYSTEMDRIVE\ferron" -Name "logs" -ItemType "directory" | Out-Null
}

# Download Ferron zip archive
$manuallyInstalled = $False
if ($installType -eq "manual")
{
    $manuallyInstalled = $True
    $ferronZipArchive = Read-Host "Path to Ferron zip archive"
} elseif ($INSTALLTYPE -eq "stable")
{
    $ferronVersion = (Invoke-RestMethod -Uri "https://dl.ferron.sh/latest2.ferron").Trim()
    if (-not $ferronVersion)
    {
        Write-Host "There was a problem while determining latest Ferron version!"
        ExitWithCode -exitcode 1
    }
    $ferronZipArchive = "$env:SYSTEMDRIVE\ferron.zip"
    Invoke-WebRequest -Uri "https://dl.ferron.sh/$ferronVersion/ferron-$ferronVersion-$triple.zip" -OutFile $ferronZipArchive
    if (-not (Test-Path $ferronZipArchive))
    {
        Write-Host "There was a problem while downloading latest Ferron version!"
        ExitWithCode -exitcode 1
    }
} else
{
    Write-Host "There was a problem determining Ferron installation type!"
    ExitWithCode -exitcode 1
}

# Check if Ferron zip archive exists
if (!(Test-Path -Path $ferronZipArchive))
{
    Write-Host "Can't find Ferron archive! Make sure to download Ferron archive file from https://ferronweb.org and rename it to 'ferron.zip'."
    ExitWithCode -exitcode 1
}

# Download WinSW
$winsw = "$env:SYSTEMDRIVE\ferron\winsw.exe"
$winswUrl = "https://github.com/winsw/winsw/releases/download/v2.12.0/WinSW-$architecture.exe"
if ($architecture -eq 'arm64')
{
    $winswUrl = "https://github.com/winsw/winsw/releases/download/v2.12.0/WinSW.NET4.exe"
}
Invoke-WebRequest -Uri $winswUrl -OutFile $winsw
if (!(Test-Path -Path $winsw))
{
    Write-Host "Can't find WinSW executable!"
    ExitWithCode -exitcode 1
}

# Extract Ferron files
Write-Host "Extracting Ferron files..."
Expand-Archive -Path $ferronZipArchive -DestinationPath "$env:SYSTEMDRIVE\ferron" -Force
if (!($manuallyInstalled))
{
    Remove-Item -Path $ferronZipArchive
}

# Create Ferron configuration
$ferronKdlConfig = @"
// See https://ferron.sh/docs/configuration-kdl for the configuration reference

// Global configuration.
//
// Here you can put configuration that applies to all hosts,
// and even to the web server itself.
globals {
  // Log requests and errors into log files
  log `"$env:SYSTEMDRIVE\\ferron\\logs\\access.log`"
  error_log `"$env:SYSTEMDRIVE\\ferron\\logs\\error.log`"
}

// Host configuration.
//
// Here you can put configuration that applies to a specific host,
// by default a catch-all `":80`" host that applies to all hostnames and port 80 (HTTP).
//
// Replace `":80`" with your domain name (pointing to your server) to use HTTPS.
// If you don't specify the paths to the TLS certificate and private key manually,
// Ferron will obtain a TLS certificate automatically (via Let's Encrypt by default).
:80 {
  // Serve static files
  root `"$env:SYSTEMDRIVE\\ferron\\wwwroot`"

  // Reverse proxy to a backend server
  //proxy `"http://localhost:3000/`"
}
"@
$ferronKdlConfig | Out-File -FilePath "$env:SYSTEMDRIVE\ferron\ferron.kdl" -Encoding ascii

# Generate WinSW Configuration
Write-Host "Generating WinSW configuration..."
$winswConfig = @"
<service>
  <id>ferron</id>
  <name>Ferron</name>
  <description>This service runs Ferron - a fast, memory-safe web server written in Rust.</description>
  <executable>$env:SYSTEMDRIVE\ferron\ferron.exe</executable>
  <arguments>-c $env:SYSTEMDRIVE\ferron\ferron.kdl</arguments>
  <log mode=`"none`"></log>
</service>
"@

$winswConfig | Out-File -FilePath "$env:SYSTEMDRIVE\ferron\winsw.xml" -Encoding ascii

# Install the service
Write-Host "Installing the Ferron service using WinSW..."
Start-Process -FilePath $winsw -ArgumentList "install" -Wait
Start-Service -Name "ferron"

# Create the uninstallation batch file
$uninstallBat = @"
@echo off
sc stop ferron
%SYSTEMDRIVE%\ferron\winsw.exe uninstall
rd %SYSTEMDRIVE%\ferron /s /q
"@

$uninstallBat | Out-File -FilePath "$env:SYSTEMDRIVE\uninstall_ferron.bat" -Encoding ascii

echo "Done! Ferron is installed successfully!"
