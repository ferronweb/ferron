# Ferron Linux installer

Self-extracting shell installer for Linux. It bundles the package-manager config (`ferron.pkgunix.conf`), prompts for an install method, and produces a single portable `dist/install.sh` that boots the installer when run as root.

## Building

```
just installer   # runs `make` in installer/
# or, manually:
make -C installer
```

The Makefile has two targets:

- `prepare` — copies `src/` into `staging/` and drops `../configs/ferron.pkgunix.conf` in as `staging/ferron.conf`.
- `bundle` — writes `dist/install.sh`. Every staging file is inlined into the script with its lines prefixed by `X` (so the script stays plain text), then extracted at runtime into `$FERRON_INSTALLER_EXTRACT_DIR` (a fresh `mktemp -d`) and `main.sh` is sourced.

On a successful exit the extraction directory is cleaned up; on a failed install the extracted scripts and log are left behind for inspection.

## Running

```
sudo ./dist/install.sh
```

The installer walks numbered steps in `src/steps/`, showing a banner, a spinner per step, and OK/FAIL transitions. It supports two non-interactive overrides:

- `FERRON_VERSION` — pin the release version instead of fetching the latest from `dl.ferron.sh`.
- `FERRON_ARCHIVE_PATH` — install from a local release tarball instead of downloading.

Install methods are offered based on what `00_preflight.sh` detects: archive download, or the distro package manager (APT/DNF) when a matching repo exists. On a machine with an existing Ferron install the installer offers update/uninstall instead.

## Structure

```
src/
├── main.sh       # entry point: loads libs, renders banner, sources numbered steps
├── lib/          # UI plumbing
│   ├── tty.sh    # terminal detection
│   ├── log.sh    # log_init / log_write
│   ├── ui.sh     # banner + spinner
│   ├── prompt.sh # interactive prompts
│   └── step.sh   # run_step spinner / OK / FAIL handling
├── steps/        # the numbered install steps (see table)
└── assets/       # banner artwork variants (and other assets)
```

### Steps

| Step                    | File                                                                                                                  | Purpose |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------- | ------- |
| `00_preflight.sh`       | host detection (root, distro, arch, libc, init system, existing install) + install-method selection                   |         |
| `10_download.sh`        | fetch the release tarball for the detected target triple and verify the SHA-256 checksum, or validate a local archive |         |
| `15_package_install.sh` | set up the APT/DNF repository and install the `ferron3` package                                                       |         |
| `20_user.sh`            | create the `ferron` system user and group                                                                             |         |
| `30_dirs.sh`            | create `/etc/ferron`, `/var/log/ferron`, `/var/lib/ferron`, `/var/www/ferron`, `/run/ferron` with correct ownership   |         |
| `40_binaries.sh`        | extract the archive and install binaries into `/usr/sbin` (with backup/restore on update)                             |         |
| `50_config.sh`          | install `ferron.conf` (only if absent) and populate `/var/www/ferron` (only if empty)                                 |         |
| `60_service.sh`         | generate and install the systemd unit / OpenRC / SysV init script, optionally enable and start it                     |         |
| `70_selinux.sh`         | SELinux contexts, booleans, and QUIC UDP ports for RHEL/Fedora systems                                                |         |
| `80_uninstall.sh`       | stop the service, remove binaries, config, data, and (optionally) the `ferron` user                                   |         |
| `90_verify.sh`          | smoke tests: binary/version check, config validation, service status, port 80, HTTP request, package presence         |         |

Package-manager installs skip the archive-specific steps (download, user, dirs, binaries, config, service, SELinux), since the package's own postinst/scripts handle those.
