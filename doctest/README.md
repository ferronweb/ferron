# Documentation testing for Ferron

This directory contains a program that would test the configuration examples (`ferron` code blocks) in the documentation.

To run it, run this command from the project root:

```bash
cargo run --manifest-path doctest/Cargo.toml
```

The program would require a Ferron binary to be built (debug binaries are preferred over release binaries in this program), as it uses the `ferron validate` command to check the configuration examples. You can build the binary with:

```bash
cargo build -p ferron
```
