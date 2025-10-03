#!/bin/bash
set -e
echo "FERRON TEST SUITE"
echo "==========================="
echo ""
pushd $(dirname $0) > /dev/null
TEST_FAILED=0
for dir in tests/*; do
    echo "Executing the '$(echo $dir | sed 's/^tests\///')' test suite..."
    echo "---------------------------"
    pushd "$dir" > /dev/null
    bash run.sh 2>&1 || TEST_FAILED=1
    popd > /dev/null
    echo ""
done
popd > /dev/null
if [ "$TEST_FAILED" -eq 1 ]; then
    echo "One or more tests failed!" >&2
    exit 1
fi
