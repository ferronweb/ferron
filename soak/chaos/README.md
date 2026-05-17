# Chaos harness

This directory contains simple mock services and orchestration to simulate hostile behaviors during soak testing.

## Files

| File | Description |
|------|-------------|
| `otlp_hang.py` | An HTTP server that accepts OTLP HTTP requests and hangs (never responds) to simulate a stuck collector. |
| `bad_backend.py` | A small TCP/HTTP server that can simulate slow backends, partial headers, malformed chunked encoding, and abrupt connection closes. |
| `run_chaos.sh` | Orchestrates a chaos run by bringing up services and exercising scenarios. |

## Usage

From the `soak/` directory:

```bash
cd soak
./chaos/run_chaos.sh
```

This script will:

1. Copy `ferron-chaos.conf` to `ferron-test.conf` for use by the compose file.
2. Use `docker-compose.chaos.yml` (which mounts the chaos scripts into the Python containers) to bring up the services.

## Scenarios

The bad backend exposes several endpoints at `http://bad-backend:8000`:

| Endpoint | Behavior |
|----------|----------|
| `/slow?delay=10` | Respond after `<delay>` seconds |
| `/close_mid` | Send partial body then close connection |
| `/partial_headers` | Slowly stream headers (slowloris-like) |
| `/malformed_chunked` | Respond with invalid chunked encoding |

## Purpose

Use this harness to validate:

- Graceful degradation under adversarial conditions
- Queue and backpressure behavior
- Logging and observability when backends or exporters misbehave
