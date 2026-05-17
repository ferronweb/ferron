#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
N=${N:-100}
INTERVAL=${INTERVAL:-0.5}

echo "Reload hammer: sending ${N} HUP signals to ferron with interval ${INTERVAL}s"
for i in $(seq 1 "${N}"); do
  timestamp=$(date -Iseconds)
  echo "[${timestamp}] sending HUP ($i/${N})"
  docker kill --signal=HUP ferron || true
  sleep "${INTERVAL}"
done
echo "Done."
