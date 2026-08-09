#!/bin/bash

PACKAGE_SCRIPT="./packaging/deb/package.sh"
if [ "${FIPS:-0}" = "1" ]; then
    PACKAGE_SCRIPT="./packaging/deb/package-fips.sh"
fi
docker run -v "$(pwd)":/ferron3 --rm debian bash -c "apt update && \
  DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends rustc git && \
  git config --global --add safe.directory /ferron3 && \
  cd /ferron3 && $PACKAGE_SCRIPT $1"
