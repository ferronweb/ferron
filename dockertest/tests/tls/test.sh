#!/bin/bash

TEST_FAILED=0

TEST_RESULTS="$(curl -fsSLk -o /dev/null https://ferron/)"
TEST_EXIT_CODE=$?
if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo "TLS connection test passed!"
else
    echo "TLS connection test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
