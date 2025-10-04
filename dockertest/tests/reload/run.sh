#!/bin/bash
TEST_FAILED=0
rm -f ferron.kdl
cat > ferron.kdl << EOF
:80 {
  root "/var/www/ferron"
}
EOF
docker compose --progress quiet up --build -d > /dev/null || (echo "Failed to start containers for a test suite" >&2; exit 1)
cat before.sh | docker compose --progress quiet exec -T test-runner bash 2>&1 || TEST_FAILED=1
cat > ferron.kdl << EOF
:80 {
  root "/var/www/ferron2"
}
EOF
docker compose --progress quiet kill -s SIGHUP ferron > /dev/null || true
cat after.sh | docker compose --progress quiet exec -T test-runner bash 2>&1 || TEST_FAILED=1
docker compose --progress quiet kill -s SIGKILL > /dev/null || true
docker compose --progress quiet down -v > /dev/null || true
rm -f ferron.kdl
if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
