# Ferron packaging

Scripts that turn release binaries into distributable artifacts. Everything ends up in `dist/` at the repository root.

All `package.sh` scripts follow the same conventions:

- **Version** — taken from `entrypoint/Cargo.toml`, falling back to the most recent git tag.
- **Target triple** — optional first argument; defaults to the host tuple (`rustc --print host-tuple`). When given, binaries are read from `target/<triple>/release`.
- **Binaries** — everything in the release directory (extensionless files plus `*.exe`, `*.dll`, `*.dylib`, `*.so`) is copied, so the whole CLI surface (`ferron`, `ferron-fmt`, `ferron-kdl2ferron`, `ferron-passwd`, `ferron-precompress`, `ferron-serve`) ships together.
- **Contents** — release binaries, the `wwwroot` web root, a systemd unit, and a packaged config (`configs/ferron.pkgunix.conf` for deb/rpm, `configs/ferron.release.conf` for the archive).

## `archive/`

Generic release archives, platform-agnostic:

- `package.sh` — Unix. Produces `dist/ferron-<version>-<triple>.tar.gz` (or `.zip` for Windows targets).
- `package.ps1` — PowerShell equivalent, using 7-Zip (with a `Compress-Archive` fallback) for Windows targets.

This is the payload that the installer's `10_download.sh` step fetches from `dl.ferron.sh`.

## `deb/`

Debian/Ubuntu packages built with `dpkg-deb`:

- `package.sh [triple]` — maps the target triple to a Debian architecture (`amd64`, `arm64`, `armhf`, `i386`, `ppc64el`, `riscv64`, `s390x`), stages the payload under `ferron3_<version>_<arch>/`, computes MD5 sums, and builds `dist/ferron3_<version>_<arch>.deb`.
- `debian/` — the package metadata and maintainer scripts: `control`, `conffiles` (`/etc/ferron/ferron.conf`), `postinst` (creates the `ferron` user, seeds `/var/www/ferron`, handles systemd), `prerm` (stops the service), and `postrm` (daemon-reload, purge).
- `ferron.service` — the shared systemd unit.
- `package-docker.sh` — wrapper that runs the build inside a `debian` container so `dpkg-deb` doesn't need to be installed on the host.

## `rpm/`

RHEL/Fedora packages built with `rpmbuild`:

- `package.sh [triple]` — maps the target triple to an RPM architecture (`x86_64`, `aarch64`, `armv7hl`, `i686`, `ppc64le`, `riscv64`, `s390x`), stages the payload under `data/`, and builds via `ferron-template.spec`.
- `ferron-template.spec` — the spec file: creates the `ferron` user in `%pre`, seeds the web root and applies SELinux contexts/booleans/QUIC ports in `%post` (reversed in `%postun`), and manages the systemd unit via `systemd-update-helper` (so the package can also be built on Debian-based systems).
- `ferron.service` — the shared systemd unit.
- `package-docker.sh` — wrapper that runs the build inside a `fedora` container with `rpm-build` and `rpmdevtools`.

## `sbom/`

SBOMs (Software Bill of Materials) using CycloneDX (XML and JSON format), packaged into archives, platform-agnostic:

- `package.sh` — Unix. Produces `dist/ferron-<version>-<triple>-sbom.tar.gz` (or `.zip` for Windows targets).
- `package.ps1` — PowerShell equivalent, using 7-Zip (with a `Compress-Archive` fallback) for Windows targets.

## `windows/`

Windows installer built with Inno Setup (Windows host only):

- `package.ps1 [triple]` — copies release binaries, `configs/ferron.pkgwin.conf`, and `wwwroot` into a `staging/` directory, then invokes `ISCC.exe` with the target triple and version baked in.
- `ferron.iss` — the Inno Setup script: installs to `{autopf}\Ferron`, offers optional PATH and Windows-service tasks (via `ferron.exe winservice install`), and stops/uninstalls the service on uninstall. Architecture selection (`x64`, `arm64`, `x86`) is derived from the target triple.
- `icon.ico`, `image.png`, `smallimage.png` — installer branding.

Output: `dist/ferron-<version>-<triple>-setup.exe`.

## Justfile shortcuts (run from project root)

```
just package [target]           # release archive (delegates to packaging/archive)
just package-deb [target]       # Debian package (uses Docker)
just package-rpm [target]       # RPM package (uses Docker)
just package-windows [target]   # Windows installer (Windows host only)
```
