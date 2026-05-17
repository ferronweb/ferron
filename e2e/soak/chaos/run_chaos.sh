#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Prepare ferron config for chaos (overwrites ferron-test.conf used by compose)
cp -v ferron-chaos.conf ferron-test.conf

# Start ferron + collector + chaos services
docker-compose -f docker-compose.yml -f docker-compose.chaos.yml up --build -d

# Give services time to start
sleep 6

DURATION=${DURATION:-60m}
CONCURRENCY=${CONCURRENCY:-50}
SCENARIOS=${SCENARIOS:-default} # comma-separated: default,slow,close_mid,partial_headers,malformed_chunked,all

# Helper to map scenario name to URL
get_url() {
  case "$1" in
    default) echo "http://ferron/";;
    slow) echo "http://ferron/slow?delay=10";;
    close_mid) echo "http://ferron/close_mid";;
    partial_headers) echo "http://ferron/partial_headers";;
    malformed_chunked) echo "http://ferron/malformed_chunked";;
    *) echo "";;
  esac
}

IFS=',' read -ra SC_ARR <<< "$SCENARIOS"

PIDS=()
LOGS_DIR="./chaos-logs"
LOGS_DIR_DISPLAY="../chaos-logs"
mkdir -p "$LOGS_DIR"

for s in "${SC_ARR[@]}"; do
  s="$(echo "$s" | xargs)"  # trim
  if [ -z "$s" ]; then
    s="default"
  fi

  if [ "$s" = "all" ]; then
    for a in default slow close_mid partial_headers malformed_chunked; do
      url="$(get_url "$a")"
      echo "Starting scenario $a -> $url"
      docker-compose -f docker-compose.yml -f docker-compose.chaos.yml run --rm loadgen -z "$DURATION" -c "$CONCURRENCY" "$url" > "$LOGS_DIR/${a}.log" 2>&1 &
      PIDS+=("$!")
    done
    continue
  fi

  url="$(get_url "$s")"
  if [ -z "$url" ]; then
    echo "Unknown scenario: $s" >&2
    exit 1
  fi

  echo "Starting scenario $s -> $url"
  docker-compose -f docker-compose.yml -f docker-compose.chaos.yml run --rm loadgen -z "$DURATION" -c "$CONCURRENCY" "$url" > "$LOGS_DIR/${s}.log" 2>&1 &
  PIDS+=("$!")
done

# Give scenarios a short warmup period
sleep 5

# Simulate OTLP sink hang: stop normal collector, bring up otlp-hang
echo "Simulating OTLP hang: stopping otel-collector and starting otlp-hang"
docker-compose -f docker-compose.yml -f docker-compose.chaos.yml stop otel-collector || true
docker-compose -f docker-compose.yml -f docker-compose.chaos.yml up -d otlp-hang || true

# Let the hang run to create backpressure
sleep 30

echo "Restoring OTLP collector"
docker-compose -f docker-compose.yml -f docker-compose.chaos.yml stop otlp-hang || true
docker-compose -f docker-compose.yml -f docker-compose.chaos.yml up -d otel-collector || true

# Wait for scenarios to finish
for pid in "${PIDS[@]}"; do
  wait "$pid" || true
done

# Print tail logs
echo "Chaos run complete; tailing ferron logs"
docker-compose -f docker-compose.yml -f docker-compose.chaos.yml logs --no-color ferron | tail -n 200

echo "Logs for scenarios are in $LOGS_DIR_DISPLAY"
echo "To stop and remove containers: docker-compose -f docker-compose.yml -f docker-compose.chaos.yml down"
