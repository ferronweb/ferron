#!/usr/bin/env bash

# Refresh pinned external inputs used by the Nix packaging:
#   1. `nix/package-bin.nix`: latest published release (`version` default
#      plus per-system hashes), following the installer's `latest3.ferron`
#      channel (see installer/src/steps/10_download.sh).
#   2. `nix/package.nix`: git submodule pins (`rev` + `hash` for the two
#      build-time submodules; compare with `git submodule status`).
#
# Idempotent: files are only rewritten when values actually change.
# Runnable locally and in CI (needs: bash, curl, git, sed, awk, nix with
# the `nix-command` experimental feature for `nix hash convert`, and
# `nix-prefetch-url`). Run from anywhere; the repository root is detected
# via git.
#
# NOTE: `#!/usr/bin/env bash` (not `#!/bin/bash` like packaging/*.sh)
# because NixOS has no /bin/bash and this script must run there too.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
PACKAGE_BIN="$REPO_ROOT/nix/package-bin.nix"
PACKAGE_SRC="$REPO_ROOT/nix/package.nix"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: required tool '$1' not found" >&2
    exit 1
  }
}
need curl
need git
need sed
need awk
need nix-prefetch-url

# `nix hash convert` needs the nix-command experimental feature on some
# setups; probe with a known-valid hash (sha256 of the empty string).
EMPTY_SRI="sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
if echo "$EMPTY_SRI" | nix hash convert --hash-algo sha256 --from sri --to sri >/dev/null 2>&1; then
  CONVERT="nix hash convert --hash-algo sha256"
else
  CONVERT="nix --extra-experimental-features nix-command hash convert --hash-algo sha256"
fi

CHANGED=0
note_change() {
  # note_change <description> <old> <new>
  if [ "$2" != "$3" ]; then
    echo "update $1: $2 -> $3"
    CHANGED=1
  fi
}

echo "==> checking latest published release..."
LATEST_VERSION="$(curl -fsSL https://dl.ferron.sh/latest3.ferron)"
echo "latest published: $LATEST_VERSION"

SUMS="$(curl -fsSL "https://dl.ferron.sh/$LATEST_VERSION/SHA256SUMS")"
WANT_X86=""
WANT_A64=""
for arch in x86_64 aarch64; do
  triple="${arch}-unknown-linux-gnu"
  hex="$(printf '%s\n' "$SUMS" | awk -v f="ferron-$LATEST_VERSION-$triple.tar.gz" '$2 == f {print $1}')"
  if [ -z "$hex" ]; then
    echo "error: no SHA256SUMS entry for ferron-$LATEST_VERSION-$triple.tar.gz" >&2
    exit 1
  fi
  # shellcheck disable=SC2034
  if [ "$arch" = x86_64 ]; then
    WANT_X86="$($CONVERT --from base16 --to sri "$hex")"
  else
    WANT_A64="$($CONVERT --from base16 --to sri "$hex")"
  fi
done

CUR_VERSION="$(sed -n 's/^  version ? "\(.*\)",$/\1/p' "$PACKAGE_BIN")"
# NOTE: anchored on the `sha256-` prefix — the `triples` map just below
# uses the same `*-linux` keys with triple values.
CUR_X86="$(sed -n 's/^    x86_64-linux = "\(sha256-.*\)";$/\1/p' "$PACKAGE_BIN")"
CUR_A64="$(sed -n 's/^    aarch64-linux = "\(sha256-.*\)";$/\1/p' "$PACKAGE_BIN")"
note_change "package-bin version" "$CUR_VERSION" "$LATEST_VERSION"
note_change "package-bin x86_64 hash" "$CUR_X86" "$WANT_X86"
note_change "package-bin aarch64 hash" "$CUR_A64" "$WANT_A64"
if [ "$CUR_VERSION" != "$LATEST_VERSION" ] || [ "$CUR_X86" != "$WANT_X86" ] || [ "$CUR_A64" != "$WANT_A64" ]; then
  sed -i "s/^  version ? \".*\",$/  version ? \"$LATEST_VERSION\",/" "$PACKAGE_BIN"
  sed -i "s|^    x86_64-linux = \"sha256-[^\"]*\";|    x86_64-linux = \"$WANT_X86\";|" "$PACKAGE_BIN"
  sed -i "s|^    aarch64-linux = \"sha256-[^\"]*\";|    aarch64-linux = \"$WANT_A64\";|" "$PACKAGE_BIN"
  echo "patched $PACKAGE_BIN"
fi

echo "==> checking git submodules..."
while IFS='|' read -r subpath owner repo; do
  [ -n "$subpath" ] || continue
  rev="$(git -C "$REPO_ROOT" ls-tree HEAD "$subpath" | awk '{print $3}')"
  if [ -z "$rev" ]; then
    echo "error: cannot resolve gitlink for $subpath (is HEAD checked out?)" >&2
    exit 1
  fi
  sri="$(nix-prefetch-url --unpack "https://github.com/$owner/$repo/archive/$rev.tar.gz" 2>/dev/null | tail -1 | xargs -I{} $CONVERT --from nix32 --to sri {})"
  range="/repo = \"$repo\";/,/^  };/"
  cur_rev="$(sed -n "$range p" "$PACKAGE_SRC" | sed -n 's/^    rev = "\(.*\)";$/\1/p')"
  cur_hash="$(sed -n "$range p" "$PACKAGE_SRC" | sed -n 's/^    hash = "\(.*\)";$/\1/p')"
  note_change "submodule $repo rev" "$cur_rev" "$rev"
  note_change "submodule $repo hash" "$cur_hash" "$sri"
  if [ "$cur_rev" != "$rev" ] || [ "$cur_hash" != "$sri" ]; then
    sed -i "$range s|^    rev = .*|    rev = \"$rev\";|" "$PACKAGE_SRC"
    sed -i "$range s|^    hash = .*|    hash = \"$sri\";|" "$PACKAGE_SRC"
  fi
done <<<"modules/observability-otlp/opentelemetry-proto|open-telemetry|opentelemetry-proto
modules/http-basicauth/phc-winner-argon2|P-H-C|phc-winner-argon2"
if [ "$CHANGED" = 1 ]; then
  echo "patched $PACKAGE_SRC (if listed above)"
fi

echo "done (changed=$CHANGED)."
