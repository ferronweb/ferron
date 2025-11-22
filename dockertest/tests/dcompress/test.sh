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

TEST_RESULTS="$(curl -fsSL -H 'Accept-Encoding: gzip' http://ferron/small.txt | gunzip)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/small.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with gzip dynamic content compression test passed!"
else
    echo "Static file serving with gzip dynamic content compression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -H 'Accept-Encoding: deflate' http://ferron/small.txt > /tmp/results.txt && \
                printf "\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\x00" | cat - /tmp/results.txt | (gzip -dc 2>/dev/null || true))"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/small.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with Deflate dynamic content compression test passed!"
else
    echo "Static file serving with Deflate dynamic content compression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -H 'Accept-Encoding: br' http://ferron/small.txt | brotli -d)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/small.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with Brotli dynamic content compression test passed!"
else
    echo "Static file serving with Brotli dynamic content compression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

TEST_RESULTS="$(curl -fsSL -H 'Accept-Encoding: zstd' http://ferron/small.txt | unzstd)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="$(cat /var/www/ferron/small.txt)"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving with Zstandard dynamic content compression test passed!"
else
    echo "Static file serving with Zstandard dynamic content compression test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
