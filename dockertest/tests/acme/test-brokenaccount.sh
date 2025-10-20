#!/bin/bash

TEST_FAILED=0

# Wait for the TLS certificate to be issued
for i in $(seq 1 90)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    curl -fsLk -o /dev/null https://ferron/ && break || true
done

TEST_RESULTS="$(curl -fsSLk -o /dev/null https://ferron/)"
TEST_EXIT_CODE=$?
if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo "Non-existent account removal from cache test passed!"
else
    echo "Non-existent account removal from cache test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
