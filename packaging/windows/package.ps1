param(
    [string]$TargetTriple = $null
)

# Get Ferron version from Cargo.toml
$CargoTomlPath = Join-Path $PSScriptRoot '../../entrypoint/Cargo.toml'
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

# Get target dir
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

$ReleasePath = Join-Path $TargetDir 'release'

if (-not (Test-Path $ReleasePath))
{
    Write-Error "Release directory not found: $ReleasePath. Build the project first."
    exit 1
}

# Create staging directory
$StagingDir = Join-Path $PSScriptRoot "staging"
if (Test-Path $StagingDir)
{ Remove-Item $StagingDir -Recurse -Force
}
New-Item -ItemType Directory -Path $StagingDir -Force | Out-Null

# Copy binaries
Write-Host "Copying binaries..."
Get-ChildItem -Path $ReleasePath -File -Filter '*.exe' | ForEach-Object {
    Copy-Item $_.FullName -Destination $StagingDir
}

# Copy configuration
$ConfigPath = Join-Path $PSScriptRoot '../../configs/ferron.pkgwin.conf'
if (Test-Path $ConfigPath)
{
    Copy-Item $ConfigPath -Destination (Join-Path $StagingDir 'ferron.conf')
}

# Copy wwwroot
$WwwrootPath = Join-Path $PSScriptRoot '../../wwwroot'
if (Test-Path $WwwrootPath)
{
    Copy-Item $WwwrootPath -Destination (Join-Path $StagingDir 'wwwroot') -Recurse -Force
}

# Create dist directory if it doesn't exist
$DistDir = Join-Path $PSScriptRoot "../../dist"
if (-not (Test-Path $DistDir))
{ New-Item -ItemType Directory -Path $DistDir | Out-Null
}

# Run Inno Setup Compiler
$ISCC = Get-Command iscc.exe -ErrorAction SilentlyContinue
if ($null -eq $ISCC)
{
    # Try common installation paths
    $ISCC_Path = "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
    if (-not (Test-Path $ISCC_Path))
    {
        Write-Warning "Inno Setup Compiler (iscc.exe) not found in PATH or default location. Staging directory prepared at: $StagingDir"
        exit 0
    }
    $ISCC = $ISCC_Path
}

Write-Host "Compiling installer..."
& $ISCC /DMyAppTargetTriple=$TargetTriple /DMyAppVersion=$FERRON_VERSION (Join-Path $PSScriptRoot "ferron.iss")

if ($LASTEXITCODE -eq 0)
{
    Write-Host "Installer created successfully in dist/"
} else
{
    Write-Error "Inno Setup compilation failed."
    exit 1
}
