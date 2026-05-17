#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
DURATION=${DURATION:-60m}
CONCURRENCY=${CONCURRENCY:-50}
SLEEP_BEFORE_START=${SLEEP_BEFORE_START:-5}

# Prepare ferron config for chaos (overwrites ferron-test.conf used by compose)
cp -v ferron-test.conf .ferron-test.conf

echo "Building and starting ferron (docker-compose)..."
docker-compose -f docker-compose.yml up --build -d ferron

echo "Waiting ${SLEEP_BEFORE_START}s for ferron to start..."
sleep "${SLEEP_BEFORE_START}"

echo "Starting load generator (duration=${DURATION}, concurrency=${CONCURRENCY})..."
docker-compose run --rm loadgen -z "${DURATION}" -c "${CONCURRENCY}" http://ferron/

echo "Soak finished; last ferron logs:"
docker-compose logs --no-color ferron | tail -n 200

echo "To stop and remove containers: docker compose down"
