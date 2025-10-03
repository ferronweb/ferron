#!/bin/bash
TEST_FAILED=0
docker compose up --build -d|| (echo "Failed to start containers for a test suite" >&2; exit 1)
cat test.sh | docker compose exec -T test-runner bash 2>&1 || TEST_FAILED=1
docker compose kill -s SIGKILL || true
docker compose down -v || true
if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
