---
title: Installation via Docker
description: "Run Ferron in Docker or Docker Compose: pull the image, start a container, verify, manage containers, and see image tags."
---

You can install Ferron via a Docker image for containerized deployments.

## Prerequisites

Before starting the installation, you need:

- A system with Docker installed. If Docker is not installed, follow the official [Docker installation guide](https://docs.docker.com/get-started/get-docker/).
- Internet connectivity to pull the Ferron Docker image.

## Installation steps

### 1. Pull the Ferron Docker image

To download the latest Ferron image from Docker Hub, run the following command:

```sh
docker pull ferronserver/ferron:3
```

### 2. Run the Ferron container

Once you download the image, start a Ferron container with the command below:

```sh
docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron:3
```

This command does the following:

- `--name myferron` - assigns a name (`myferron`) to the running container.
- `-d` - runs the container in detached mode (as a background process).
- `-p 80:80` - maps port 80 of the container to port 80 on the host machine.
- `--restart=always` - makes sure the container automatically restarts if it stops or if the system reboots.

## Verifying the installation

To confirm that Ferron runs, execute:

```sh
docker ps
```

This should display a running container with the name `myferron`.

To test the web server, open a browser and navigate to `http://localhost`. If you see the `Ferron is installed successfully!` message on the page, the web server works correctly.

You can also use `curl` instead:

```sh
curl http://localhost
```

## File structure

Ferron on Docker has the following file structure:

- `/usr/local/bin/ferron` - Ferron web server
- `/usr/local/bin/ferron-fmt` - Ferron configuration formatter
- `/usr/local/bin/ferron-kdl2ferron` - Ferron configuration conversion tool
- `/usr/local/bin/ferron-passwd` - Ferron user password generation tool
- `/usr/local/bin/ferron-precompress` - Ferron static files precompression tool
- `/usr/local/bin/ferron-serve` - command for serving static files with Ferron with zero configuration
- `/var/cache/ferron-acme` - the ACME cache directory for Ferron (if not explicitly specified in the server configuration)
- `/var/www/ferron` - the default web root for Ferron
- `/etc/ferron/conf.d` - Directory for split Ferron configuration files
- `/etc/ferron/conf.d/00-default.conf` - Default Ferron configuration

## Managing the Ferron container

### Stopping the container

To stop the Ferron container, run:

```sh
docker stop myferron
```

### Restarting the container

To restart the container:

```sh
docker start myferron
```

### Removing the container

If you need to remove the Ferron container:

```sh
docker rm -f myferron
```

### Viewing the logs

To view the Ferron access and error logs, use the Docker `logs` command with the container name or ID:

```sh
docker logs myferron
```

> [!tip]
> By default, the Ferron Docker image outputs structured JSON-format access logs. These logs carry `grep`-able trace IDs that correlate with error logs.

## Using Ferron with Docker Compose

If you use Docker Compose, you can define a service for Ferron in your `docker-compose.yml` file:

```yaml
services:
  ferron:
    image: ferronserver/ferron:3
    ports:
      - "80:80"
    restart: always
```

Then, you can start Ferron using:

```sh
docker compose up -d
```

### Example: Ferron with Docker Compose and automatic TLS

If using Ferron with Docker Compose and automatic TLS, you can use the following `docker-compose.yml` file contents:

```yaml
services:
  # Ferron container
  ferron:
    image: ferronserver/ferron:3
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - "./ferron-conf.d:/etc/ferron/conf.d" # Ferron configuration file
      - "ferron-acme:/var/cache/ferron-acme" # This volume is needed for persistent automatic TLS cache, otherwise the web server will obtain a new certificate on each restart
    restart: always

volumes:
  ferron-acme:
```

You might also configure Ferron in a `ferron-conf.d/ferron.conf` file like this:

```ferron
# Replace "example.com" with your website's domain name
example.com {
    root "/var/www/ferron"
}
```

Then, you can start Ferron using:

```sh
docker compose up -d
```

## Ferron image tags

The Ferron 3 image has the following tags:

- `3` - Based on Distroless, statically-linked binaries
- `3-alpine` - Based on Alpine Linux, statically-linked binaries
- `3-debian` - Based on Debian GNU/Linux, dynamically-linked binaries (GNU libc required)

## Images with FIPS-certified cryptography

Ferron publishes FIPS-certified image variants for deployments that require FIPS-approved cryptography. These use the same base images but with the `-fips` suffix on the tag. For example:

- `3-fips`
- `3-alpine-fips`
- `3-debian-fips`

Pull and run a FIPS image the same way as the standard images:

```sh
docker pull ferronserver/ferron:3-fips
docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron:3-fips
```

FIPS images restrict cryptography to FIPS-approved algorithms: TLS cipher suites and key exchange groups are filtered, and HTTP basic auth password verification accepts only PBKDF2 hashes (Argon2 and scrypt are rejected).

If you build the Ferron image yourself, pass the `FIPS=1` build argument to enable the FIPS build:

```sh
docker build --build-arg FIPS=1 -t ferronserver/ferron:3-fips .
```

> [!note]
> Use FIPS image variants when you must run Ferron in a FIPS-compliant environment. The standard images use a broader set of algorithms and are not FIPS-certified.
