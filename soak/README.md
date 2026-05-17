# Soak tests

Soak test scaffolding for Ferron. These scripts exercise Ferron under sustained load to validate stability, resource usage, and graceful degradation over time.

## Quick start

Build and run Ferron with a load generator:

```bash
cd soak
./run_soak.sh
```

## Configuration

Customize the test duration and concurrency:

```bash
DURATION=360m CONCURRENCY=100 ./run_soak.sh
```

| Variable       | Description                         | Default |
|----------------|-------------------------------------|---------|
| `DURATION`     | Total test duration (e.g., `360m`)  | `60m`   |
| `CONCURRENCY`  | Number of concurrent connections    | `50`    |

## Reload hammer

Send repeated `SIGHUP` signals to the Ferron container to test configuration reload under load:

```bash
cd soak
./reload_hammer.sh
```

## Notes

- These scripts are designed for local development with Docker. For long-running lab jobs, run them on a dedicated host or CI runner.
- The Docker Compose file builds Ferron from the repository root using `Dockerfile.alpine` and exposes port `8080` on the host.
- Ensure Docker and Docker Compose are installed and accessible before running.
