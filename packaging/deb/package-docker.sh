#!/bin/bash

docker run -v "$(pwd)":/ferron3 --rm -ti debian bash -c "apt update && \
  DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends rustc git && \
  git config --global --add safe.directory /ferron3 && \
  cd /ferron3 && ./packaging/deb/package.sh $1"
