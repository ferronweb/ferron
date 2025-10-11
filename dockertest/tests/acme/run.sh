#!/bin/bash
TEST_FAILED=0
docker run -v ./cache:/tmp/cache --rm alpine chmod -R a+rwx /tmp/cache
rm -rf cache
mkdir -p cache
docker compose --progress quiet up --build -d > /dev/null || (echo "Failed to start containers for a test suite" >&2; exit 1)
cat test.sh | docker compose --progress quiet exec -T test-runner bash 2>&1 || TEST_FAILED=1
docker compose --progress quiet restart > /dev/null || true # This also removes the ACME accounts
cat test-cache.sh | docker compose --progress quiet exec -T test-runner bash 2>&1 || TEST_FAILED=1
docker run -v ./cache:/tmp/cache --rm alpine find /tmp/cache -mindepth 1 -maxdepth 1 -type d ! -name 'account_*' ! -name 'hostname_*' -exec rm -rf {} \; # Remove certificate cache
docker compose --progress quiet restart > /dev/null || true # Restart containers after removing cache
cat test-brokenaccount.sh | docker compose --progress quiet exec -T test-runner bash 2>&1 || TEST_FAILED=1
docker compose --progress quiet kill -s SIGKILL > /dev/null || true
docker compose --progress quiet down -v > /dev/null || true
docker run -v ./cache:/tmp/cache --rm alpine chmod -R a+rwx /tmp/cache
rm -rf cache
if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
