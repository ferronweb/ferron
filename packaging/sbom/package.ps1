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

Write-Host "Using version: $FERRON_VERSION"

# Get target triple from argument or use host triple
if ([string]::IsNullOrEmpty($TargetTriple))
{
    $TargetTriple = rustc --print host-tuple 2>$null

    if ([string]::IsNullOrEmpty($TargetTriple))
    {
        Write-Error "Failed to get host triple from rustc"
        exit 1
    }
}

Write-Host "Target triple: $TargetTriple"

# Remove old SBOMs
Get-ChildItem -Path . -Recurse -File -Filter '*.cdx.json' | ForEach-Object {
    Remove-Item $_.FullName -Force
}
Get-ChildItem -Path . -Recurse -File -Filter '*.cdx.xml' | ForEach-Object {
    Remove-Item $_.FullName -Force
}

# Invoke cargo cyclonedx
cargo cyclonedx -f json --describe binaries --target "$TargetTriple"
cargo cyclonedx -f xml --describe binaries --target "$TargetTriple"

# Create a temporary directory for packaging
$TempDir = [System.IO.Path]::GetTempPath() + [System.Guid]::NewGuid().ToString()
New-Item -ItemType Directory -Path $TempDir -Force | Out-Null

# Copy SBOMs to temporary directory
Get-ChildItem -Path . -Recurse -File -Filter '*.cdx.json' | ForEach-Object {
    Copy-Item $_.FullName -Destination $TempDir
}
Get-ChildItem -Path . -Recurse -File -Filter '*.cdx.xml' | ForEach-Object {
    Copy-Item $_.FullName -Destination $TempDir
}

# Save previous directory
$PreviousDir = Get-Location

# Create dist directory
$DistDir = Join-Path $PreviousDir 'dist'
New-Item -ItemType Directory -Path $DistDir -Force | Out-Null

# Determine filename and create archive
$FilenameNoExt = Join-Path $DistDir "ferron-$FERRON_VERSION-$TargetTriple-sbom"

if ($TargetTriple -match 'windows')
{
    # For Windows, create a ZIP archive
    $Filename = "$FilenameNoExt.zip"
    Remove-Item $Filename -ErrorAction SilentlyContinue
    Set-Location $TempDir
    # Use 7zip if available, otherwise fall back to Compress-Archive
    if (Get-Command 7z -ErrorAction SilentlyContinue)
    {
        7z a $Filename .\*
    } else
    {
        # Try common installation paths
        $PossiblePaths = @(
            'C:\Program Files\7-Zip\7z.exe'
            'C:\Program Files (x86)\7-Zip\7z.exe'
        )
        $FoundPath = $null
        foreach ($Path in $PossiblePaths)
        {
            if (Test-Path $Path)
            {
                $FoundPath = $Path
                break
            }
        }
        if ($FoundPath)
        {
            & $FoundPath a $Filename .\*
        } else
        {
            Write-Host "7zip not found, falling back to Compress-Archive..."
            # In PowerShell 5.1, Compress-Archive uses "\" for path separators instead of "/".
            # This is a known issue with these versions of PowerShell.
            Compress-Archive -Path .\* -DestinationPath $Filename -Force
        }
    }
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
