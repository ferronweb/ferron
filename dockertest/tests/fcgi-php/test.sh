#!/bin/bash

TEST_FAILED=0

TEST_RESULTS="$(curl -fsSL http://ferron/)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="Hello, World!"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Basic PHP-FPM test passed!"
else
    echo "Basic PHP-FPM test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
