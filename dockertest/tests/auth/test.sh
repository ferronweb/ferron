#!/bin/bash

TEST_FAILED=0

TEST_RESULTS="$(curl -u test:test -fsSL http://ferron/test.txt)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/test.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "HTTP authentication test passed!"
else
    echo "HTTP authentication test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -u test:fail -fsL -w %{http_code} http://ferron/test.txt || true)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="401"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "HTTP authentication failure test passed!"
else
    echo "HTTP authentication failure test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
