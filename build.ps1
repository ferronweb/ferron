# Get version from Cargo.toml
$ferronVersionCargo = Select-String -Path "ferron/Cargo.toml" -Pattern '^version' | ForEach-Object {
    if ($_ -match '"([0-9a-zA-Z.+-]+)"') {
        $matches[1]
    }
}

# Get version from latest git tag
$ferronVersionGit = git tag --sort=-committerdate | Select-Object -First 1 | ForEach-Object {
    $_ -replace '[^0-9a-zA-Z.+-]', ''
}

# Use Cargo version if available, otherwise fallback to git version
$ferronVersion = if ($ferronVersionCargo) { $ferronVersionCargo } else { $ferronVersionGit }

# Get host target triple
$hostTargetTriple = rustc -vV | Where-Object { $_ -match '^host: ' } | ForEach-Object { $_ -replace 'host: ', '' }

# Check if $env:TARGET is set
if ($env:TARGET) {
    $cargoFinalExtraArgs = "--target $env:TARGET"
    $cargoTargetRoot = "target/$env:TARGET"
    $destTargetTriple = $env:TARGET
    $buildRelease = "build-release-$env:TARGET"
} else {
    $cargoFinalExtraArgs = ""
    $cargoTargetRoot = "target"
    $destTargetTriple = $hostTargetTriple
    $buildRelease = "build-release"
}

# Set cargo executable if not set
if (-not $env:CARGO_FINAL) {
    $cargoFinal = "cargo"
} else {
    $cargoFinal = $env:CARGO_FINAL
}

function Run {
    Build
    & "$cargoTargetRoot/release/ferron"
}

function RunDev {
    BuildDev
    & "$cargoTargetRoot/debug/ferron"
}

function Build {
    PrepareBuild
    Push-Location build-workspace
    & cargo update
    & $cargoFinal build --target-dir ../target -r $cargoFinalExtraArgs
    Pop-Location
}

function BuildDev {
    PrepareBuild
    Push-Location build-workspace
    & cargo update
    & $cargoFinal build --target-dir ../target $cargoFinalExtraArgs
    Pop-Location
}

function PrepareBuild {
    & cargo run --manifest-path build-prepare/Cargo.toml
}

function Package {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $buildRelease
    New-Item -ItemType Directory -Path $buildRelease | Out-Null

    Get-ChildItem "$cargoTargetRoot/release" -File |
        Where-Object { !$_.Name.Contains('.') -or $_.Extension -in ".exe", ".dll", ".dylib", ".so" } |
        ForEach-Object {
            Copy-Item -Path $_.FullName -Destination $buildRelease -Force
        }

    Copy-Item ferron-release.kdl "$buildRelease/ferron.kdl" -Force
    Copy-Item wwwroot -Destination $buildRelease -Recurse -Force

    if (-not (Test-Path "dist")) { New-Item -ItemType Directory -Path "dist" | Out-Null }
	if (Test-Path "dist/ferron-$ferronVersion-$destTargetTriple.zip") { Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "dist/ferron-$ferronVersion-$destTargetTriple.zip" }
    Compress-Archive -Path "$buildRelease\*" -DestinationPath "dist/ferron-$ferronVersion-$destTargetTriple.zip"

    Remove-Item -Recurse -Force $buildRelease
}

function BuildWithPackage {
    Build
    Package
}

function Clean {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue build-workspace, build-release, dist
    & cargo clean
}

if ($args.Count -gt 0) {
    & $args[0]
} else {
    Write-Host "Available commands: Run, RunDev, Build, BuildDev, PrepareBuild, Package, BuildWithPackage, Clean"
}
