# Binary-download variant of the Ferron package.
#
# Fetches the release archive from the Ferron download server, with this
# URL:
#   https://dl.ferron.sh/{version}/ferron-{version}-{triple}.tar.gz
# where `version` would be the latest published release (fetched from
# https://dl.ferron.sh/latest3.ferron) and `triple` would be the target
# triple of the host (like `x86_64-unknown-linux-gnu` for 64-bit x86
# Linux)
#
# Checksums are published at https://dl.ferron.sh/{version}/SHA256SUMS
#
# Refresh hashes using `nix/update-pins.sh` (which is run automatically
# when Ferron is released).
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  version ? "3.0.0-rc.10",

  # `releaseHashes`, not `hashes` because `pkgs.hashes` exists.
  releaseHashes ? {
    x86_64-linux = "sha256-GqAO4/NYnm2lB+TuzWyDywVneNw8ag4vgs07+U0FXII=";
    aarch64-linux = "sha256-folfK/X5jHHK1KzGgtHVn5yi/gjhuE4ObmiZ3CxD3fQ=";
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

  # Search for dynamically-linked dependencies and patch them
  # (pre-built Ferron uses only GNU libc and companions anyway)
  nativeBuildInputs = [ autoPatchelfHook ];
  buildInputs = [ stdenv.cc.cc.lib ];

  # Without this, stdenv's unpackPhase would pick wwwroot/ (a webroot!)
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
