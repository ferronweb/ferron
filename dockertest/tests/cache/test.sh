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

# Test 1: Long cache (Hit)
echo "v1" > /var/www/ferron/long/test.txt
# Prime the cache
curl -fsSL http://ferron/long/test.txt > /dev/null
# Modify file
echo "v2" > /var/www/ferron/long/test.txt
# Request again
TEST_RESULTS="$(curl -fsSL http://ferron/long/test.txt)"
TEST_EXPECTED="v1"
if [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Cache hit test passed!"
else
    echo "Cache hit test failed!" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

# Test 2: Short cache (Expiry)
echo "v1" > /var/www/ferron/short/test.txt
# Prime the cache
curl -fsSL http://ferron/short/test.txt > /dev/null
# Modify file
echo "v2" > /var/www/ferron/short/test.txt
# Wait for expiry (max-age=2)
sleep 3
# Request again
TEST_RESULTS="$(curl -fsSL http://ferron/short/test.txt)"
TEST_EXPECTED="v2"
if [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Cache expiration test passed!"
else
    echo "Cache expiration test failed!" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

# Test 3: Vary header
echo "v1" > /var/www/ferron/vary/test.txt
# Prime cache for Header A
curl -fsSL -H "X-Test-Header: A" http://ferron/vary/test.txt > /dev/null
# Modify file
echo "v2" > /var/www/ferron/vary/test.txt
# Request with Header B (should miss and get v2)
TEST_RESULTS="$(curl -fsSL -H "X-Test-Header: B" http://ferron/vary/test.txt)"
TEST_EXPECTED="v2"
if [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Cache vary (miss) test passed!"
else
    echo "Cache vary (miss) test failed!" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi
# Request with Header A (should hit and get v1)
TEST_RESULTS="$(curl -fsSL -H "X-Test-Header: A" http://ferron/vary/test.txt)"
TEST_EXPECTED="v1"
if [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Cache vary (hit) test passed!"
else
    echo "Cache vary (hit) test failed!" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
