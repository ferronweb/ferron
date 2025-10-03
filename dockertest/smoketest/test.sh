#!/bin/bash

TEST_FAILED=0

TEST_RESULTS="$(curl -fsL -w %{http_code} -o /dev/null http://ferron || true)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="200"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Smoke test passed!"
else
    echo "Smoke test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
