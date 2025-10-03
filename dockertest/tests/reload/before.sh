#!/bin/bash

TEST_FAILED=0

TEST_RESULTS="$(curl -fsSL http://ferron/test.txt)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/test.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Before reloading configuration test passed!"
else
    echo "Before reloading configuration test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
