#!/bin/bash
TEST_FAILED=0
cp hello.proto backend/
cp hello.proto grpcweb-test-proxy/
docker compose --progress quiet up --build -d > /dev/null || (echo "Failed to start containers for a test suite" >&2; exit 1)
cat test.sh | docker compose --progress quiet exec -T test-runner bash 2>&1 || TEST_FAILED=1
docker compose --progress quiet kill -s SIGKILL > /dev/null || true
docker compose --progress quiet down -v > /dev/null || true
rm -f backend/hello.proto
rm -f grpcweb-test-proxy/hello.proto
if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
