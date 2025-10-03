#!/bin/bash
TEST_FAILED=0
rm -rf certs
mkdir -p certs
docker run -t -i --rm -v ./certs:/etc/certs alpine/openssl req -x509 -newkey rsa:4096 -keyout /etc/certs/server.key -out /etc/certs/server.crt -days 365 -nodes -subj "/CN=localhost" > /dev/null || (echo "Failed to generate TLS key pair for a test suite" >&2; exit 1)
docker compose --progress quiet up --build -d > /dev/null || (echo "Failed to start containers for a test suite" >&2; exit 1)
cat test.sh | docker compose --progress quiet exec -T test-runner bash 2>&1 || TEST_FAILED=1
docker compose --progress quiet kill -s SIGKILL > /dev/null || true
docker compose --progress quiet down -v > /dev/null || true
rm -rf certs
if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
