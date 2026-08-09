set windows-shell := ["powershell.exe", "-c"]

# Build the project
build fips="false":
    cargo build -r {{ if fips == "true" { "--features=fips" } else { "" } }}

# Cross-build the optimized binaries for the project (+ optional PGO)
[linux]
cross-build target pgo="false" fips="false":
    ./cross-build/build.sh {{ target }} {{ if fips == "true" { "--fips" } else { "" } }} {{ if pgo == "true" { "--pgo" } else { "" } }}

# Run the project for testing
run:
    cargo run --bin ferron

# Prepare the configuration file for testing
[unix]
prepare-config:
    cp configs/ferron.conf.example ferron.conf

# Prepare the configuration file for testing
[windows]
prepare-config:
    copy configs/ferron.conf.example ferron.conf

# Package the release binaries
[unix]
package target="" fips="false":
    {{ if fips == "true" { "./packaging/archive/package-fips.sh" } else { "./packaging/archive/package.sh" } }} {{ target }}

# Package the release binaries
[windows]
package target="" fips="false":
    powershell -ExecutionPolicy Bypass -File {{ if fips == "true" { "packaging/archive/package-fips.ps1" } else { "packaging/archive/package.ps1" } }} {{ target }}

# Package the release binaries as a Debian package
package-deb target="" fips="false":
    {{ if fips == "true" { "FIPS=1" } else { "" } }} ./packaging/deb/package-docker.sh {{ target }}

# Package the release binaries as an RPM package
package-rpm target="" fips="false":
    {{ if fips == "true" { "FIPS=1" } else { "" } }} ./packaging/rpm/package-docker.sh {{ target }}

# Package the release binaries as a Windows installer
[windows]
package-windows target="" fips="false":
    powershell -ExecutionPolicy Bypass -File {{ if fips == "true" { "packaging/windows/package-fips.ps1" } else { "packaging/windows/package.ps1" } }} {{ target }}

# Generate SBOMs and package them using `cargo cyclonedx`
[unix]
package-sbom target="" fips="false":
    {{ if fips == "true" { "./packaging/sbom/package-fips.sh" } else { "./packaging/sbom/package.sh" } }} {{ target }}

# Generate SBOMs and package them using `cargo cyclonedx`
[windows]
package-sbom target="" fips="false":
    powershell -ExecutionPolicy Bypass -File {{ if fips == "true" { "packaging/sbom/package-fips.ps1" } else { "packaging/sbom/package.ps1" } }} {{ target }}

# Build the installer for Linux
[unix]
installer:
    cd installer && make
