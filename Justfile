# Build the project
build:
    cargo build

# Prepare the configuration file for testing
[unix]
prepare-config:
    cp configs/ferron.conf.example ferron.conf

# Prepare the configuration file for testing
[windows]
prepare-config:
    copy configs/ferron.conf.example ferron.conf
