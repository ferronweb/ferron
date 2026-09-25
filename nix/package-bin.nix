# Binary-download variant of the Ferron package.
#
# Fetches the release archive produced by `just package` (see
# packaging/archive/package.sh) from dl.ferron.sh, mirroring what the
# installer's 10_download.sh step does:
#   https://dl.ferron.sh/{version}/ferron-{version}-{triple}.tar.gz
# with checksums published at https://dl.ferron.sh/{version}/SHA256SUMS
#
# `version` tracks the latest *published* release (like the installer's
# `latest3.ferron` channel), deliberately decoupled from the checkout's
# Cargo.toml version so the default stays installable between version
# bumps and publishes. Refresh via `nix/update-pins.sh` (also wired to a
# scheduled GitHub Actions workflow) — never by hand.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  version ? "3.0.0-rc.7",
  # NOTE: named `releaseHashes` (not `hashes`) because `pkgs.hashes`
  # exists -- `callPackage` would inject it over a `hashes` default.
  releaseHashes ? {
    x86_64-linux = "sha256-gs2rGWXSYJoxlM/OLzyEvqfXXuTxw4vrsRv6GPtPfvQ=";
    aarch64-linux = "sha256-joCXQW3IZKkHew8xQTHcrr9Ca74+ZRDvXgQBiFav+x0=";
  },
}:

let
  triples = {
    x86_64-linux = "x86_64-unknown-linux-gnu";
    aarch64-linux = "aarch64-unknown-linux-gnu";
  };

  triple =
    triples.${stdenv.hostPlatform.system}
      or (throw "unsupported system: ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation {
  pname = "ferron-bin";
  inherit version;

  src = fetchurl {
    url = "https://dl.ferron.sh/${version}/ferron-${version}-${triple}.tar.gz";
    sha256 = releaseHashes.${stdenv.hostPlatform.system};
  };

  # Prebuilt binaries target generic Linux (dynamic glibc). Patch the
  # interpreter/RPATH so they run on NixOS without a manual loader;
  # required for `services.ferron.package = ...ferron-bin` to work.
  nativeBuildInputs = [ autoPatchelfHook ];

  # Runtime libs for autoPatchelf to link (the release binaries need
  # libgcc_s; glibc comes from stdenv).
  buildInputs = [ stdenv.cc.cc.lib ];

  # The archive is flat (binaries + ferron.conf at top level, plus a
  # wwwroot/ dir). Without this, stdenv's unpackPhase would pick wwwroot/
  # as sourceRoot and installPhase would run in the wrong directory.
  sourceRoot = ".";

  # The archive contains the binaries, ferron.conf and wwwroot at top level
  # (see packaging/archive/package.sh).
  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin $out/share/ferron
    for bin in ferron ferron-fmt ferron-kdl2ferron ferron-passwd ferron-precompress ferron-serve; do
      if [ -f "$bin" ]; then
        install -m0755 "$bin" $out/bin/
      fi
    done
    if [ -f ferron.conf ]; then
      cp ferron.conf $out/share/ferron/ferron.conf.example
    fi
    if [ -d wwwroot ]; then
      cp -r wwwroot $out/share/ferron/wwwroot
    fi
    runHook postInstall
  '';

  meta = with lib; {
    description = "Ferron web server (prebuilt binaries)";
    homepage = "https://ferron.sh";
    license = licenses.mit;
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
    mainProgram = "ferron";
  };
}
