#!/bin/bash

TEST_FAILED=0

# Wait for the backend servers to start
for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z backend-1 3000 >/dev/null 2>&1 && break || true
done

for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z backend-2 3000 >/dev/null 2>&1 && break || true
done

for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z backend-3 3000 >/dev/null 2>&1 && break || true
done

# Wait for the HTTP server to start
for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z ferron 80 >/dev/null 2>&1 && break || true
done

TEST_RESULTS="$(curl -fsSL http://backend-1:3000/)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="Hello, World!"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Backend server #1 smoke test passed!"
else
    echo "Backend server #1 smoke test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://backend-2:3000/)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="Hello, World!"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Backend server #2 smoke test passed!"
else
    echo "Backend server #2 smoke test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://backend-3:3000/)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="Hello, World!"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Backend server #3 smoke test passed!"
else
    echo "Backend server #3 smoke test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://ferron-random/ && echo -n " " && curl -fsSL http://ferron-random/ && echo -n " " && curl -fsSL http://ferron-random/)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="Hello, World! Hello, World! Hello, World!"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Basic load balancing (random selection) test passed!"
else
    echo "Basic load balancing (random selection) test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL http://ferron-round-robin/ && echo -n " " && curl -fsSL http://ferron-round-robin/ && echo -n " " && curl -fsSL http://ferron-round-robin/)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="Hello, World! Hello, World! Hello, World!"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Basic load balancing (round-robin) test passed!"
else
    echo "Basic load balancing (round-robin) test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
