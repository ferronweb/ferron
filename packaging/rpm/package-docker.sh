#!/bin/bash
#
PACKAGE_SCRIPT="./packaging/rpm/package.sh"
if [ "${FIPS:-0}" = "1" ]; then
    PACKAGE_SCRIPT="./packaging/rpm/package-fips.sh"
fi
docker run -v "$(pwd)":/ferron3 --rm fedora bash -c "dnf makecache -y && \
  dnf install -y rustc git rpm-build rpmdevtools systemd-rpm-macros && \
  git config --global --add safe.directory /ferron3 && \
  cd /ferron3 && $PACKAGE_SCRIPT $1"
