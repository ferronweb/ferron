# Ferron manual testing setup

This directory contains a Docker Compose-based environment for manually testing Ferron features, including:

- static file serving,
- automatic TLS with Pebble (Let's Encrypt test server),
- HTTP/3,
- PHP via FastCGI,
- observability with OpenTelemetry, Prometheus, Loki, and Grafana.

This setup is intended to run on GNU/Linux systems with Docker and Docker Compose installed.

## How to set up the testing environment?

1. Add test hostnames to your hosts file:

```text
127.0.0.1 ferron.test grafana.ferron.test php.ferron.test
```

2. Start the environment from the project root directory:

```bash
docker compose -f manualtest/docker-compose.yml up --build -d
```

3. Check that all containers are running:

```bash
docker compose -f manualtest/docker-compose.yml ps
```

## How to use the testing environment?

### Endpoints

- Ferron (HTTP): `http://ferron.test`
- Ferron (HTTPS): `https://ferron.test`
- PHP test page: `https://php.ferron.test`
- Grafana (proxied by Ferron): `https://grafana.ferron.test`

### Grafana login

- Username: `admin`
- Password: `admin`

### Useful checks

Check HTTP response:

```bash
curl -v http://ferron.test
```

Check HTTPS response (Pebble certificate is not trusted by default):

```bash
curl -vk https://ferron.test
```

Check HTTP/3 support:

```bash
curl --http3-only -vk https://ferron.test
```

Check PHP through FastCGI:

```bash
curl -vk https://php.ferron.test
```

Follow Ferron logs:

```bash
docker compose -f manualtest/docker-compose.yml logs -f ferron
```

## How to tear down the testing environment?

Stop and remove containers and networks:

```bash
docker compose -f manualtest/docker-compose.yml down
```

To also remove pulled/built images used by this setup:

```bash
docker compose -f manualtest/docker-compose.yml down --rmi local
```
