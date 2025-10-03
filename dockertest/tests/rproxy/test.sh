#!/bin/bash

TEST_FAILED=0

sleep 0.5 # Sleep before making requests to allow backend to start
TEST_RESULTS="$(curl -fsSL http://backend:3000/)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="Hello, World!"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Backend server smoke test passed!"
else
    echo "Backend server smoke test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://ferron/)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="Hello, World!"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Basic reverse proxy test passed!"
else
    echo "Basic reverse proxy test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(echo "WEBSOCKET TEST" | websocat ws://ferron/echo)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="WEBSOCKET TEST"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "WebSocket reverse proxy test passed!"
else
    echo "WebSocket reverse proxy test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://ferron/ip)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(ip route get "$(host ferron | awk 'NR==1 {print $NF}')" \
                 | grep -o "dev .* src [0-9]*\.[0-9]*\.[0-9]*\.[0-9]*" | cut -d' ' -f4)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "X-Forwarded-For test passed!"
else
    echo "X-Forwarded-For test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://ferron/hostname)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="backend:3000"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Backend hostname setting test passed!"
else
    echo "Backend hostname setting test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://ferron/header)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="something"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Custom backend header setting test passed!"
else
    echo "Custom backend header setting test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsL -w %{http_code} http://ferron/unsafe || true)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="502"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Bad gateway test passed!"
else
    echo "Bad gateway test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
