param(
    [string]$TargetTriple = $null
)

# Get Ferron version from Cargo.toml
$CargoTomlPath = Join-Path $PSScriptRoot '../../ferron/Cargo.toml'
if (Test-Path $CargoTomlPath)
{
    $CargoContent = Get-Content $CargoTomlPath -Raw
    $FERRON_VERSION_CARGO = [regex]::Match($CargoContent, 'version\s*=\s*"([^"]+)"').Groups[1].Value
} else
{
    $FERRON_VERSION_CARGO = $null
}

# Get version from most recent git tag
$FERRON_VERSION_GIT = if (Get-Command git -ErrorAction SilentlyContinue)
{
    $tags = git tag --sort=-committerdate | Select-Object -First 1
    if ($tags)
    {
        $tags -replace '[^0-9a-zA-Z.+-]', ''
    }
} else
{
    $null
}

# Determine final version
if ([string]::IsNullOrEmpty($FERRON_VERSION_CARGO))
{
    $FERRON_VERSION = $FERRON_VERSION_GIT
} else
{
    $FERRON_VERSION = $FERRON_VERSION_CARGO
}

$TargetDir = Join-Path $PSScriptRoot "../../target/$TargetTriple"

Write-Host "Using version: $FERRON_VERSION"

# Get target triple from argument or use host triple
if ([string]::IsNullOrEmpty($TargetTriple))
{
    $TargetTriple = rustc --print host-tuple 2>$null
    $TargetDir = Join-Path $PSScriptRoot "../../target"

    if ([string]::IsNullOrEmpty($TargetTriple))
    {
        Write-Error "Failed to get host triple from rustc"
        exit 1
    }
}

Write-Host "Target triple: $TargetTriple"
Write-Host "Target dir: $TargetDir"

# Create a temporary directory for packaging
$TempDir = [System.IO.Path]::GetTempPath() + [System.Guid]::NewGuid().ToString()
New-Item -ItemType Directory -Path $TempDir -Force | Out-Null

# Copy release files to temporary directory
$ReleasePath = Join-Path $TargetDir 'release'

if (Test-Path $ReleasePath)
{
    # Copy files without extension
    Get-ChildItem -Path $ReleasePath -File | Where-Object { $_.Extension -eq "" } | ForEach-Object {
        Copy-Item $_.FullName -Destination $TempDir
    }

    # Copy .exe files
    Get-ChildItem -Path $ReleasePath -File -Filter '*.exe' | ForEach-Object {
        Copy-Item $_.FullName -Destination $TempDir
    }

    # Copy .dll files
    Get-ChildItem -Path $ReleasePath -File -Filter '*.dll' | ForEach-Object {
        Copy-Item $_.FullName -Destination $TempDir
    }

    # Copy .dylib files (macOS)
    Get-ChildItem -Path $ReleasePath -File -Filter '*.dylib' | ForEach-Object {
        Copy-Item $_.FullName -Destination $TempDir
    }

    # Copy .so files (Linux)
    Get-ChildItem -Path $ReleasePath -File -Filter '*.so' | ForEach-Object {
        Copy-Item $_.FullName -Destination $TempDir
    }
}

# Copy configuration file
$ConfigPath = Join-Path $PSScriptRoot '../../configs/ferron.release.conf'
if (Test-Path $ConfigPath)
{
    Copy-Item $ConfigPath -Destination (Join-Path $TempDir 'ferron.conf')
}

# Copy wwwroot directory
$WwwrootPath = Join-Path $PSScriptRoot '../../wwwroot'
if (Test-Path $WwwrootPath)
{
    Copy-Item $WwwrootPath -Destination (Join-Path $TempDir 'wwwroot') -Recurse -Force
}

# Save previous directory
$PreviousDir = Get-Location

# Create dist directory
$DistDir = Join-Path $PreviousDir 'dist'
New-Item -ItemType Directory -Path $DistDir -Force | Out-Null

# Determine filename and create archive
$FilenameNoExt = Join-Path $DistDir "ferron-$FERRON_VERSION-$TargetTriple"

if ($TargetTriple -match 'windows')
{
    # For Windows, create a ZIP archive
    $Filename = "$FilenameNoExt.zip"
    Remove-Item $Filename -ErrorAction SilentlyContinue
    Set-Location $TempDir
    Compress-Archive -Path .\* -DestinationPath $Filename -Force
    Set-Location $PreviousDir
} else
{
    # For other platforms, create a tar.gz archive
    $Filename = "$FilenameNoExt.tar.gz"
    Remove-Item $Filename -ErrorAction SilentlyContinue
    Set-Location $TempDir
    tar -czf $Filename .\*
    Set-Location $PreviousDir
}

Write-Host "Archive created: $Filename"

# Clean up temporary directory
Remove-Item $TempDir -Recurse -Force
