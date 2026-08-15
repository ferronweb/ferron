# Ferron Cross-Compilation Build System

Cross-compilation build files for Linux targets, runnable on Linux hosts of any distro.

## Quick Start

```bash
# 1. Prepare sysroots (one-time, cacheable in CI/CD)
./cross-build/sysroots/prepare-gnu.sh --all
./cross-build/sysroots/prepare-musl.sh --all

# 2. Build for a target
./cross-build/build.sh x86_64-unknown-linux-gnu
./cross-build/build.sh aarch64-unknown-linux-musl

# 3. Build with PGO (optional)
./cross-build/build.sh aarch64-unknown-linux-gnu --pgo
```

## Prerequisites

### Required

- **Rust toolchain** with `rustup`
- **curl** (for Alpine package downloads, cross-compiled PGO, and HTTP/3)
- **clang**
- **lld**

### Optional (for PGO)

- **wrk** — HTTP benchmarking tool
- **h2load** — HTTP/2 benchmarking tool
- **openssl** — for self-signed TLS certificates
- **Python 3** — for the reverse proxy backend server
- **llvm-profdata** — for merging PGO profiles
- **qemu-user-static** — for running cross-compiled binaries

### Install by distro

**Arch Linux:**

```bash
sudo pacman -S lld clang openssl python llvm qemu-user-static qemu-user-static-binfmt curl
# h2load and wrk are in AUR if not found:
yay -S nghttp2 wrk
```

**Debian/Ubuntu:**

```bash
sudo apt install lld clang libclang-dev wrk nghttp2-client openssl python3 llvm curl qemu-user-static
```

**Fedora:**

```bash
sudo dnf install lld clang clang-devel wrk nghttp2-client openssl python3 llvm curl qemu-user-static
```

## Target Matrix

| Rust Target                      | Libc  | Status    |
| -------------------------------- | ----- | --------- |
| `x86_64-unknown-linux-gnu`       | glibc | Supported |
| `i686-unknown-linux-gnu`         | glibc | Supported |
| `aarch64-unknown-linux-gnu`      | glibc | Supported |
| `armv7-unknown-linux-gnueabihf`  | glibc | Supported |
| `riscv64gc-unknown-linux-gnu`    | glibc | Supported |
| `s390x-unknown-linux-gnu`        | glibc | Supported |
| `powerpc64le-unknown-linux-gnu`  | glibc | Supported |
| `x86_64-unknown-linux-musl`      | musl  | Supported |
| `i686-unknown-linux-musl`        | musl  | Supported |
| `aarch64-unknown-linux-musl`     | musl  | Supported |
| `armv7-unknown-linux-musleabihf` | musl  | Supported |
| `riscv64gc-unknown-linux-musl`   | musl  | Supported |

## Directory Structure

```
cross-build/
├── README.md                     # This file
├── build.sh                      # Main build orchestration
├── sysroots/
│   ├── prepare-gnu.sh            # Debian oldoldstable sysroot (glibc + libstdc++)
│   └── prepare-musl.sh           # Alpine latest sysroot (musl + libc++)
└── benchmarks/
    ├── run.sh                    # PGO training benchmarks (wrk + h2load)
    └── scenarios/
        ├── static-small.lua      # wrk Lua: small static file requests
        ├── static-large.lua      # wrk Lua: large static file requests
        └── proxy-http1.lua       # wrk Lua: reverse proxy HTTP/1.1
```

## Sysroot Preparation

### GNU targets (glibc)

Uses `debootstrap` to create a Debian buster sysroot with glibc and libstdc++:

```bash
# Single target
./cross-build/sysroots/prepare-gnu.sh x86_64-unknown-linux-gnu

# All GNU targets
./cross-build/sysroots/prepare-gnu.sh --all

# Custom output directory
./cross-build/sysroots/prepare-gnu.sh -o /opt/sysroots aarch64-unknown-linux-gnu
```

The sysroot is created at `cross-build/sysroots/gnu-<arch>/` and contains:

- GNU libc headers and libraries
- libstdc++ headers and libraries

### musl targets

Downloads Alpine packages directly (no Docker required). The musl toolchain is
fully clang/lld-based: GCC CRT startup objects (`crtbeginS.o`, `crtendS.o`) and
`libgcc.a` are taken from Alpine's `libgcc-static` package (built for the target
architecture), so **no host GCC is required**.

```bash
# Single target
./cross-build/sysroots/prepare-musl.sh x86_64-unknown-linux-musl

# All musl targets
./cross-build/sysroots/prepare-musl.sh --all

# Custom Alpine version
./cross-build/sysroots/prepare-musl.sh -v 3.24 aarch64-unknown-linux-musl
```

The sysroot is created at `cross-build/sysroots/musl-<arch>/` and contains:

- musl headers
- libc++ static libraries
- libc++ headers
- target-architecture GCC runtime objects (`crtbeginS.o`, `crtendS.o`) and `libgcc.a`

## Building

### Basic build

```bash
./cross-build/build.sh <target>

# Examples:
./cross-build/build.sh x86_64-unknown-linux-gnu
./cross-build/build.sh aarch64-unknown-linux-musl
```

### With custom sysroot

```bash
./cross-build/build.sh --sysroot-dir /path/to/sysroot <target>
```

### Output

Binaries are placed in `dist/<target>/`:

```
dist/aarch64-unknown-linux-gnu/
├── ferron
├── ferron-fmt
├── ferron-passwd
├── ferron-precompress
├── ferron-kdl2ferron
├── ferron-serve
├── ferron.conf
└── wwwroot/
```

## Profile-Guided Optimization (PGO)

PGO can improve performance by 10-20% by optimizing based on actual runtime behavior.

### Build with PGO

```bash
./cross-build/build.sh <target> --pgo
```

This runs a 4-phase process:

1. **Instrumented build** — Compiles with `-Cprofile-generate`
2. **Training benchmarks** — Runs wrk/h2load scenarios to generate profile data
3. **Profile merge** — Merges `.profraw` files with `llvm-profdata`
4. **Optimized build** — Compiles with `-Cprofile-use`

### PGO workflow details

#### Phase 1: Instrumented build

```bash
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" cargo build -r --target <target>
```

#### Phase 2: Training benchmarks

The benchmark script (`benchmarks/run.sh`) runs 5 scenarios:

| Scenario           | Tool   | Protocol      | Purpose                        |
| ------------------ | ------ | ------------- | ------------------------------ |
| Small static files | wrk    | HTTP/1.1      | Tests small response handling  |
| Large static files | wrk    | HTTP/1.1      | Tests large response streaming |
| Reverse proxy      | wrk    | HTTP/1.1      | Tests proxy request forwarding |
| Reverse proxy      | h2load | HTTP/2 + TLS  | Tests multiplexed connections  |
| Reverse proxy      | curl   | HTTP/3 + QUIC | Tests QUIC connections         |

For cross-compiled targets, the benchmark script uses `curl` instead of `wrk` and `h2load`.

#### Phase 3: Profile merge

```bash
llvm-profdata merge -output=merged.profdata /tmp/pgo-data/*.profraw
```

#### Phase 4: Optimized build

```bash
RUSTFLAGS="-Cprofile-use=merged.profdata" cargo build -r --target <target>
```

### Cross-compiled PGO

For cross-compiled targets, the benchmark script uses `qemu-user-static` to run the binary on the host. It will be installed automatically if missing.

### Custom benchmark duration

```bash
./cross-build/build.sh <target> --pgo --bench-duration 60
```

## Troubleshooting

### "Sysroot not found"

Prepare the sysroot first:

```bash
./cross-build/sysroots/prepare-gnu.sh <target>
# or
./cross-build/sysroots/prepare-musl.sh <target>
```

### "qemu-user-static not found"

Install it:

```bash
# Arch
sudo pacman -S qemu-user-static qemu-user-static-binfmt
# Debian/Ubuntu
sudo apt install qemu-user-static
```

### "clang not found"

```bash
# Arch
sudo pacman -S clang
# Debian/Ubuntu
sudo apt install clang
```

### Build fails with linker errors

Make sure the sysroot contains the required libraries:

```bash
ls -la cross-build/sysroots/gnu-<arch>/lib/
ls -la cross-build/sysroots/musl-<arch>/lib/
```
