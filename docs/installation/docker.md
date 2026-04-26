---
title: Installation via Docker
description: "Run Ferron in Docker or Docker Compose: pull the image, start a container, verify, manage containers, and see available image tags."
---

Ferron can be installed via a Docker image, for containerized deployments.

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

Once the image is downloaded, start a Ferron container using the command below:

```sh
docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron:3
```

This command does the following:

- `--name myferron` - assigns a name (`myferron`) to the running container.
- `-d` - runs the container in detached mode (as a background process).
- `-p 80:80` - maps port 80 of the container to port 80 on the host machine.
- `--restart=always` - ensures the container automatically restarts if it stops or if the system reboots.

## Verifying the installation

To confirm that Ferron is running, execute:

```sh
docker ps
```

This should display a running container with the name `myferron`.

To test the web server, open a browser and navigate to `http://localhost`. If you see a "Ferron is installed successfully!" message on the page, the web server is installed successfully and is up and running.

You can also use `curl` instead:

```sh
curl http://localhost
```

## File structure

Ferron on Docker has the following file structure:

- `/usr/local/bin/ferron` - Ferron web server
- `/usr/local/bin/ferron-kdl2ferron` - Ferron configuration conversion tool
- `/usr/local/bin/ferron-passwd` - Ferron user password generation tool
- `/usr/local/bin/ferron-precompress` - Ferron static files precompression tool
- `/usr/local/bin/ferron-serve` - command for serving static files with Ferron with zero configuration
- `/var/cache/ferron-acme` - Ferron's ACME cache directory (if not explicitly specified in the server configuration)
- `/var/log/ferron/access.log` - Ferron access log in Combined Log Format (default configuration)
- `/var/log/ferron/error.log` - Ferron error log (default configuration)
- `/var/www/ferron` - Ferron's web root
- `/etc/ferron/ferron.conf` - Ferron configuration

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

## Using Ferron with Docker Compose

If you're using Docker Compose, you can define a service for Ferron in your `docker-compose.yml` file:

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
      - "./ferron.conf:/etc/ferron/ferron.conf" # Ferron configuration file
      - "ferron-acme:/var/cache/ferron-acme" # This volume is needed for persistent automatic TLS cache, otherwise the web server will obtain a new certificate on each restart
    restart: always

volumes:
  ferron-acme:
```

You might also configure Ferron in a `ferron.conf` file like this:

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

Ferron 3 provides the following tags for the Ferron image:

- `3` - Based on Distroless, statically-linked binaries
- `3-alpine` - Based on Alpine Linux, statically-linked binaries
- `3-debian` - Based on Debian GNU/Linux, dynamically-linked binaries (GNU libc required)
