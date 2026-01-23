#!/bin/bash

TEST_FAILED=0

# Wait for the backend server to start
for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z backend 50051 >/dev/null 2>&1 && break || true
done

# Wait for the HTTP server to start
for i in $(seq 1 3)
do
    if [ "$i" -gt 1 ]; then
        sleep 1
    fi
    nc -z ferron 80 >/dev/null 2>&1 && break || true
done

TEST_RESULTS="$(grpcweb-cli --data '{"name": "Ferron"}' --include /tmp --proto /tmp/hello.proto --url http://ferron/helloworld.Greeter/SayHello | jq .message)"
TEST_EXPECTED='"Hello Ferron"'
TEST_EXIT_CODE=$?
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Basic gRPC-Web proxying test passed!"
else
    echo "Basic gRPC-Web proxying test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
