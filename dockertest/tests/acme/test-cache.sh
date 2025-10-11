#!/bin/bash

TEST_FAILED=0

# Wait for the HTTP server to start
for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z ferron 443 >/dev/null 2>&1 && break || true
done

TEST_RESULTS="$(curl -fsSLk -o /dev/null https://ferron-ondemand/)"
TEST_EXIT_CODE=$?
if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo "Cached on-demand automatic TLS (TLS-ALPN-01 ACME challenge) connection test passed!"
else
    echo "Cached on-demand automatic TLS (TLS-ALPN-01 ACME challenge) connection test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSLk -o /dev/null https://ferron-http01-ondemand/)"
TEST_EXIT_CODE=$?
if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo "Cached on-demand automatic TLS (HTTP-01 ACME challenge) connection test passed!"
else
    echo "Cached on-demand automatic TLS (HTTP-01 ACME challenge) connection test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
