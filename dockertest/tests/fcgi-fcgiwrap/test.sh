#!/bin/bash

TEST_FAILED=0

# Wait for the HTTP server to start
for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z ferron 80 >/dev/null 2>&1 && break || true
done

# Wait for the PHP-FPM daemon to start
for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z php-fpm 9000 >/dev/null 2>&1 && break || true
done

TEST_RESULTS="$(curl -fsSL http://ferron/cgi-bin/index.cgi)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="Hello, World!"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Basic fcgiwrap test passed!"
else
    echo "Basic fcgiwrap test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
