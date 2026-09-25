# Source build of Ferron from this repository.
#
# Version is read from entrypoint/Cargo.toml (same source of truth as
# packaging/archive/package.sh and packaging/deb|rpm/package.sh).
# Dependencies come from Cargo.lock at the repository root — no separate
# vendor hash is maintained (see the NOTE at `cargoLock` below).
{
  lib,
  rustPlatform,
  pkg-config,
  cmake,
  perl,
  stdenv,
  fetchFromGitHub,
}:

let
  manifest = lib.importTOML ../entrypoint/Cargo.toml;

  # Git submodules are not part of the flake's git-tracked source.
  # BUT, they would be automatically synced in CI/CD

  # Needed by modules/observability-otlp/build.rs (protobuf definitions).
  otlp-proto = fetchFromGitHub {
    owner = "open-telemetry";
    repo = "opentelemetry-proto";
    rev = "0e66a7c05a693204fa5e9660659e224017f431c5";
    hash = "sha256-d2KMYZP19WpFAbcr/+WdESq1sTJejU4Z8fRTHglvq2U=";
  };
  # Needed by modules/http-basicauth/build.rs (argon2 C sources).
  argon2 = fetchFromGitHub {
    owner = "P-H-C";
    repo = "phc-winner-argon2";
    rev = "f57e61e19229e23c4445b85494dbf7c07de721cb";
    hash = "sha256-AK7ENx1zChILb9wACxKcPHXmo4Hosytj2OBGppYW4W4=";
  };
in
rustPlatform.buildRustPackage {
  pname = "ferron";
  version = manifest.package.version;

  # Plain `../.` is the flake's git-tracked tree: untracked/ignored files
  # (target/, dist/, .git/) are already excluded, so no filter needed.
  # What the tracked tree lacks is git submodule contents (only gitlinks),
  # which the two build scripts need — hence the stitching below. Keeping
  # submodules as explicit fetches (rather than builtins.path disk copies)
  # makes the package build from any clean checkout, lockfile, or tarball
  # without requiring --recurse-submodules on disk.
  src = stdenv.mkDerivation {
    name = "ferron-source";
    src = ../.;
    dontBuild = true;
    dontConfigure = true;
    installPhase = ''
      cp -r $src $out
      chmod -R u+w $out
      rm -rf $out/modules/observability-otlp/opentelemetry-proto
      cp -r ${otlp-proto} $out/modules/observability-otlp/opentelemetry-proto
      rm -rf $out/modules/http-basicauth/phc-winner-argon2
      cp -r ${argon2} $out/modules/http-basicauth/phc-winner-argon2
      chmod -R u+w $out
    '';
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  # NOTE: no `cargoHash` here on purpose. With `cargoLock.lockFile` set and
  # no git dependencies in Cargo.lock (all 500+ deps are registry sources),
  # nixpkgs vendors purely from the lockfile, reproducible without a
  # separate vendor hash to maintain. Dependency changes flow through
  # Cargo.lock alone, so Ferron version bumps never touch this file.

  # aws-lc-rs uses the `bindgen` feature (see types/tls/Cargo.toml and
  # modules using aws-lc-rs), so libclang must be available at build time.
  # bindgenHook sets LIBCLANG_PATH automatically. aws-lc-sys also needs
  # cmake (and perl) to configure its C build.
  nativeBuildInputs = [
    pkg-config
    cmake
    perl
    rustPlatform.bindgenHook
  ];

  # Build the main binary plus the CLI utilities shipped in release
  # archives (packaging/README.md): ferron-fmt, ferron-kdl2ferron,
  # ferron-passwd, ferron-precompress, ferron-serve.
  cargoBuildFlags = [
    "-p"
    "ferron"
    "-p"
    "ferron-fmt"
    "-p"
    "ferron-kdl2ferron"
    "-p"
    "ferron-passwd"
    "-p"
    "ferron-precompress"
    "-p"
    "ferron-serve"
  ];

  # Tests need network / Docker in places; skip `cargo test` here.
  # Unit tests run via `cargo test --workspace`, E2E via `cd e2e && cargo test`
  # (see CONTRIBUTING.md).
  doCheck = false;

  postInstall = ''
    mkdir -p $out/share/ferron
    cp ${../configs/ferron.pkgunix.conf} $out/share/ferron/ferron.conf.example
    cp -r ${../wwwroot} $out/share/ferron/wwwroot
  '';

  meta = with lib; {
    description = "Ferron web server";
    homepage = "https://ferron.sh";
    license = licenses.mit;
    maintainers = [ ];
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
    mainProgram = "ferron";
  };
}
