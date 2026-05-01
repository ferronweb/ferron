#!/bin/bash

docker run -v "$(pwd)":/ferron3 --rm fedora bash -c "dnf makecache -y && \
  dnf install -y rustc git rpm-build rpmdevtools systemd-rpm-macros && \
  git config --global --add safe.directory /ferron3 && \
  cd /ferron3 && ./packaging/rpm/package.sh $1"
