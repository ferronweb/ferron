# Ferron packaging

Scripts that turn release binaries into distributable artifacts. Everything ends up in `dist/` at the repository root.

## `archive/`

Generic release archives, platform-agnostic:

Run one of these:

- `package.sh`: Unix. Produces `dist/ferron-<version>-<triple>.tar.gz` (or `.zip` for Windows targets).
- `package.ps1`: PowerShell equivalent, using 7-Zip (with a `Compress-Archive` fallback) for Windows targets.

This is the payload that the installer script for Linux fetches from `dl.ferron.sh`.

## `deb/`

Debian/Ubuntu packages built with `dpkg-deb`.

Run `package.sh [triple]` (where `triple` would be the Rust target triple Ferron was compiled with) to build the packages. If you're not using Debian, run `package-docker.sh [triple]` instead (it uses Docker).

The built package is in `dist/ferron3_<version>_<arch>.deb`.

## `rpm/`

RHEL/Fedora packages built with `rpmbuild`:

Run `package.sh [triple]` (where `triple` would be the Rust target triple Ferron was compiled with) to build the packages. If you're not using Fedora, run `package-docker.sh [triple]` instead (it uses Docker).

## `sbom/`

SBOMs (Software Bill of Materials) using CycloneDX (XML and JSON format), packaged into archives, platform-agnostic:

- `package.sh`: Unix. Produces `dist/ferron-<version>-<triple>-sbom.tar.gz` (or `.zip` for Windows targets).
- `package.ps1`: PowerShell equivalent, using 7-Zip (with a `Compress-Archive` fallback) for Windows targets.

## `windows/`

Windows installer built with Inno Setup (Windows host only):

Run `package.ps1 [triple]` (where `triple` would be the Rust target triple Ferron was compiled with) to build the installer using Inno Setup.

Output: `dist/ferron-<version>-<triple>-setup.exe`.

## Nix

Nix packaging lives outside this directory, next to the root `flake.nix`.

See the [NixOS installation guide](https://ferron.sh/docs/installation/linux/nixos) for usage. Justfile shortcuts: `just package-nix [package]`, `just update-nix-pins`.

## Justfile shortcuts (run from project root)

```
just package [target]           # release archive (delegates to packaging/archive)
just package-deb [target]       # Debian package (uses Docker)
just package-rpm [target]       # RPM package (uses Docker)
just package-windows [target]   # Windows installer (Windows host only)
```
