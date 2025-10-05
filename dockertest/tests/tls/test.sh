#!/bin/bash

TEST_FAILED=0

# Wait for the backend server to start
for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z backend 3000 >/dev/null 2>&1 && break || true
done

TEST_RESULTS="$(curl -fsSLk --http1.1 -o /dev/null https://ferron/)"
TEST_EXIT_CODE=$?
if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo "HTTP/1.1 via TLS connection test passed!"
else
    echo "HTTP/1.1 via TLS connection test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSLk --http2-prior-knowledge -o /dev/null https://ferron/)"
TEST_EXIT_CODE=$?
if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo "HTTP/2 via TLS connection test passed!"
else
    echo "HTTP/2 via TLS connection test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSLk --http3-only -o /dev/null https://ferron/)"
TEST_EXIT_CODE=$?
if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo "HTTP/3 via TLS connection test passed!"
else
    echo "HTTP/3 via TLS connection test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
