# Build the project
build:
    cargo build -r

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
package target="":
    ./packaging/archive/package.sh {{ target }}

# Package the release binaries
[windows]
package target="":
    powershell -ExecutionPolicy Bypass -File packaging/archive/package.ps1 {{ target }}

# Package the release binaries as a Debian package
package-deb target="":
    ./packaging/deb/package-docker.sh {{ target }}
