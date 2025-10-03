# Ferron test suite

This directory contains the test suite for Ferron web server. It includes a set of tests that verify the functionality of Ferron's HTTP server, automatic TLS functionality, and other features.

These tests are intended to run on GNU/Linux systems with Docker installed.

## How to run tests?

To run the tests, run the following command in the project root directory:

```bash
./dockertest/test.sh
```

## How to write tests?

A test suite is defined in a subdirectory of a `tests` directory in the `dockertest` directory. The test suite contains `run.sh` file that runs the tests in the suite. The `run.sh` file is responsible for setting up the test environment, running the tests, and cleaning up after the tests.

A test suite also contains a `docker-compose.yml` file that defines the Docker containers required for the test suite.

Example `run.sh` file:

```bash
#!/bin/bash
TEST_FAILED=0
docker compose --progress quiet up --build -d > /dev/null || (echo "Failed to start containers for a test suite" >&2; exit 1)
cat test.sh | docker compose --progress quiet exec -T test-runner bash 2>&1 || TEST_FAILED=1
docker compose --progress quiet kill -s SIGKILL > /dev/null || true
docker compose --progress quiet down -v > /dev/null || true
if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
```

Example `docker-compose.yml` file:

```yaml
services:
  # The web server to test
  ferron:
    build:
      context: ../../..

  # A container to run tests
  test-runner:
    build:
      context: ../../runner
    command: "tail -f /dev/null"
```

Example `test.sh` file (referenced from the `run.sh` example):

```bash
#!/bin/bash

TEST_FAILED=0

TEST_RESULTS="$(curl -fsL -w %{http_code} -o /dev/null http://ferron || true)"
TEST_EXIT_CODE=$?
TEST_EXPECTED="200"
if [ "$TEST_EXIT_CODE" -eq 0 ] && [ "$TEST_RESULTS" = "$TEST_EXPECTED" ]; then
    echo "Static file serving smoke test passed!"
else
    echo "Static file serving smoke test failed!" >&2
    echo "  Exit code: $TEST_EXIT_CODE" >&2
    echo "  Expected: $TEST_EXPECTED" >&2
    echo "  Received: $TEST_RESULTS" >&2
    TEST_FAILED=1
fi

if [ "$TEST_FAILED" -eq 1 ]; then
    exit 1
fi
```
