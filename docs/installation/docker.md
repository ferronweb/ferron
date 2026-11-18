---
title: Installation via Docker
description: "Run Ferron in Docker or Docker Compose: pull the image, start a container, verify, manage containers, and see image tags."
---

Ferron ships as a Docker image for containerized deployments.

## Prerequisites

- Docker on the host. See the official [Docker installation guide](https://docs.docker.com/get-started/get-docker/) if you need it.
- Internet access to pull the Ferron image.

## Installation steps

### 1. Pull the Ferron Docker image

```sh
docker pull ferronserver/ferron:3
```

### 2. Run the Ferron container

```sh
docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron:3
```

The flags:

- `--name myferron`: names the container `myferron`.
- `-d`: runs the container in the background.
- `-p 80:80`: maps container port 80 to host port 80.
- `--restart=always`: restarts the container after a stop or a reboot.

## Verifying the installation

```sh
docker ps
```

The output lists a running container named `myferron`.

Open `http://localhost` in a browser. The page shows "Ferron is ready to serve." when the server works.

You can also use `curl`:

```sh
curl http://localhost
```

## File structure

The Docker image contains these paths:

- `/usr/local/bin/ferron`: Ferron web server
- `/usr/local/bin/ferron-fmt`: configuration formatter
- `/usr/local/bin/ferron-kdl2ferron`: configuration conversion tool
- `/usr/local/bin/ferron-passwd`: user password generation tool
- `/usr/local/bin/ferron-precompress`: static file precompression tool
- `/usr/local/bin/ferron-serve`: static file serving with no configuration
- `/var/cache/ferron-acme`: ACME cache directory, unless the configuration sets another path
- `/var/www/ferron`: default web root
- `/etc/ferron/conf.d`: directory for split configuration files
- `/etc/ferron/conf.d/00-default.conf`: default configuration

## Managing the Ferron container

Stop the container:

```sh
docker stop myferron
```

Start it again:

```sh
docker start myferron
```

Remove the container:

```sh
docker rm -f myferron
```

View the access and error logs:

```sh
docker logs myferron
```

> [!tip]
> The Ferron Docker image writes structured JSON access logs by default. The logs carry trace IDs that correlate with error logs and are easy to search with `grep`.

## Using Ferron with Docker Compose

Define a Ferron service in `docker-compose.yml`:

```yaml
services:
  ferron:
    image: ferronserver/ferron:3
    ports:
      - "80:80"
    restart: always
```

Start it:

```sh
docker compose up -d
```

### Example: Ferron with Docker Compose and automatic TLS

This `docker-compose.yml` serves HTTPS with automatic TLS:

```yaml
services:
  ferron:
    image: ferronserver/ferron:3
    ports:
      - "80:80"
      - "443:443"
      - "443:443/udp" # HTTP/3 + QUIC
    volumes:
      - "./ferron-conf.d:/etc/ferron/conf.d"
      - "ferron-acme:/var/cache/ferron-acme"
    restart: always

volumes:
  ferron-acme:
```

The `ferron-acme` volume keeps the ACME cache. Without it, the server obtains a new certificate on each restart.

A configuration in `ferron-conf.d/ferron.conf`:

```ferron
example.com {
    root /var/www/ferron
}
```

Start it:

```sh
docker compose up -d
```

## Ferron image tags

The Ferron 3 image has these tags:

- `3`: Distroless base, statically linked binaries.
- `3-alpine`: Alpine Linux base, statically linked binaries.
- `3-debian`: Debian GNU/Linux base, dynamically linked binaries that need GNU libc.

## Images with FIPS-certified cryptography

FIPS-certified image variants use the same base images with a `-fips` suffix:

- `3-fips`
- `3-alpine-fips`
- `3-debian-fips`

Pull and run them the same way:

```sh
docker pull ferronserver/ferron:3-fips
docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron:3-fips
```

FIPS images restrict cryptography to FIPS-approved algorithms:

- OCSP stapling, TLS cipher suites, and key exchange groups are filtered.
- HTTP basic auth password verification accepts only PBKDF2 hashes. Argon2, scrypt, and bcrypt are rejected.

To build a FIPS image yourself, pass the `FIPS=1` build argument:

```sh
docker build --build-arg FIPS=1 -t ferronserver/ferron:3-fips .
```

> [!note]
> Use a FIPS image when you must run Ferron in a FIPS-compliant environment. Standard images support more algorithms and are not FIPS-certified.
