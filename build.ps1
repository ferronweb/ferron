# Get version from Cargo.toml
$ferronVersionCargo = Select-String -Path "ferron/Cargo.toml" -Pattern '^version' | ForEach-Object {
    if ($_ -match '"([0-9a-zA-Z.+-]+)"')
    {
        $matches[1]
    }
}

# Get version from latest git tag
$ferronVersionGit = git tag --sort=-committerdate | Select-Object -First 1 | ForEach-Object {
    $_ -replace '[^0-9a-zA-Z.+-]', ''
}

# Use Cargo version if available, otherwise fallback to git version
$ferronVersion = if ($ferronVersionCargo)
{ $ferronVersionCargo
} else
{ $ferronVersionGit
}

# Get host target triple
$hostTargetTriple = rustc -vV | Where-Object { $_ -match '^host: ' } | ForEach-Object { $_ -replace 'host: ', '' }

# Check if $env:TARGET is set
if ($env:TARGET)
{
    $cargoFinalExtraArgs = "--target $env:TARGET"
    $cargoTargetRoot = "target/$env:TARGET"
    $destTargetTriple = $env:TARGET
    $buildRelease = "build-release-$env:TARGET"
} else
{
    $cargoFinalExtraArgs = ""
    $cargoTargetRoot = "target"
    $destTargetTriple = $hostTargetTriple
    $buildRelease = "build-release"
}

# Append extra arguments if set
if ($env:CARGO_FINAL_EXTRA_ARGS)
{
    $cargoFinalExtraArgs = "$cargoFinalExtraArgs $env:CARGO_FINAL_EXTRA_ARGS"
}

# There would be a static file serving performance degradation when using Monoio, so we're compiling with Tokio
$cargoFinalExtraArgs = "--no-default-features -F ferron/runtime-tokio $cargoFinalExtraArgs"

# Split the arguments (to avoid being interpreted as one large argument)
$cargoFinalExtraArgs = $cargoFinalExtraArgs -Split ' '

# Set cargo executable if not set
if (-not $env:CARGO_FINAL)
{
    $cargoFinal = "cargo"
} else
{
    $cargoFinal = $env:CARGO_FINAL
}

function Run
{
    Build
    & "$cargoTargetRoot/release/ferron"
}

function RunDev
{
    BuildDev
    & "$cargoTargetRoot/debug/ferron"
}

function Smoketest
{
    Build
    $env:FERRON = $PWD.Path + '\' + $cargoTargetRoot + '\release\ferron'
    & powershell.exe -ExecutionPolicy Bypass ".\smoketest\smoketest.ps1"
}

function SmoketestDev
{
    BuildDev
    $env:FERRON = $PWD.Path + '\' + $cargoTargetRoot + '\debug\ferron'
    & powershell.exe -ExecutionPolicy Bypass ".\smoketest\smoketest.ps1"
}

function Build
{
    PrepareBuild
    FixConflicts
    Push-Location build-workspace
    & $cargoFinal build --target-dir ../target -r $cargoFinalExtraArgs
    Pop-Location
}

function BuildDev
{
    PrepareBuild
    FixConflicts
    Push-Location build-workspace
    & $cargoFinal build --target-dir ../target $cargoFinalExtraArgs
    Pop-Location
}

function PrepareBuild
{
    & cargo run --manifest-path build-prepare/Cargo.toml
}

function FixConflicts
{
    Push-Location build-workspace
    while (($oldConflictingPackages -ne $conflictingPackages) -or (-not $oldConflictingPackages))
    {
        $oldConflictingPackages = $conflictingPackages
        $conflictingPackages = (cargo update -w --dry-run 2>&1) | Select-String -Pattern '^error: failed to select a version for (?:the requirement )?`([^ `]+)' | ForEach-Object { $_.Matches.Groups[1].Value }
        $conflictingPackages = $conflictingPackages -Split ' '
        if (-not $conflictingPackages)
        { break
        }
        if ($oldConflictingPackages -eq $conflictingPackages)
        { throw "Couldn't resolve Cargo conflicts"
        }
        if ($conflictingPackages)
        { & cargo update $conflictingPackages
        }
    }
    Pop-Location
}

function Package
{
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $buildRelease
    New-Item -ItemType Directory -Path $buildRelease | Out-Null

    Get-ChildItem "$cargoTargetRoot/release" -File |
        Where-Object { !$_.Name.Contains('.') -or $_.Extension -in ".exe", ".dll", ".dylib", ".so" } |
        ForEach-Object {
            Copy-Item -Path $_.FullName -Destination $buildRelease -Force
        }

    Copy-Item ferron-release.kdl "$buildRelease/ferron.kdl" -Force
    Copy-Item wwwroot -Destination $buildRelease -Recurse -Force

    if (-not (Test-Path "dist"))
    { New-Item -ItemType Directory -Path "dist" | Out-Null
    }
    if (Test-Path "dist/ferron-$ferronVersion-$destTargetTriple.zip")
    { Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "dist/ferron-$ferronVersion-$destTargetTriple.zip"
    }

    Compress-Archive -Path "$buildRelease\*" -DestinationPath "dist/ferron-$ferronVersion-$destTargetTriple.zip"

    Remove-Item -Recurse -Force $buildRelease
}

function BuildWithPackage
{
    Build
    Package
}

function Clean
{
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue build-workspace, build-release, dist, packaging/deb/ferron_* packaging/deb/md5sums.tmp
    & cargo clean
    Push-Location build-prepare
    & cargo clean
    Pop-Location
}

if ($args.Count -gt 0)
{
    & $args[0]
} else
{
    Write-Host "Available commands: Run, RunDev, Build, BuildDev, Smoketest, SmoketestDev, PrepareBuild, FixConflicts, Package, BuildWithPackage, Clean"
}
