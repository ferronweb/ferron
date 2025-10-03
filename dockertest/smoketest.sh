#!/bin/bash
set -e
pushd "$(dirname $0)/smoketest" > /dev/null
bash run.sh || exit 1
popd > /dev/null
