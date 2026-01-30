---
title: Installation via Docker
---

## Prerequisites

Before starting the installation, you need:

- A system with Docker installed. If Docker is not installed, follow the official [Docker installation guide](https://docs.docker.com/get-started/get-docker/).
- Internet connectivity to pull the Ferron Docker image.

## Installation steps

### 1. Pull the Ferron Docker image

To download the latest Ferron image from Docker Hub, run the following command:

```sh
docker pull ferronserver/ferron:2
```

### 2. Run the Ferron container

Once the image is downloaded, start a Ferron container using the command below:

```sh
docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron:2
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

Ferron on Docker has following file structure:

- _/usr/sbin/ferron_ - Ferron web server
- _/usr/sbin/ferron-passwd_ - Ferron user password generation tool
- _/usr/sbin/ferron-precompress_ - Ferron static files precompression tool
- _/usr/sbin/ferron-yaml2kdl_ - Ferron configuration conversion tool
- _/var/cache/ferron-acme_ - Ferron's ACME cache directory (if not explicitly specified in the server configuration)
- _/var/log/ferron/access.log_ - Ferron access log in Combined Log Format (default configuration)
- _/var/log/ferron/error.log_ - Ferron error log (default configuration)
- _/var/www/ferron_ - Ferron's web root
- _/etc/ferron.kdl_ - Ferron configuration

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
    image: ferronserver/ferron:2
    ports:
      - "80:80"
    restart: always
```

Then, you can start Ferron using:

```sh
docker-compose up -d
```

### Example: Ferron with Docker Compose and automatic TLS

If using Ferron with Docker Compose and automatic TLS, you can use the following `docker-compose.yml` file contents:

```yaml
services:
  # Ferron container
  ferron:
    image: ferronserver/ferron:2
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - "./ferron.kdl:/etc/ferron.kdl" # Ferron configuration file
      - "ferron-acme:/var/cache/ferron-acme" # This volume is needed for persistent automatic TLS cache, otherwise the web server will obtain a new certificate on each restart
    restart: always

volumes:
  ferron-acme:
```

You might also configure Ferron in a "ferron.kdl" file like this:

```kdl
// Replace "example.com" with your website's domain name
example.com {
    root "/var/www/ferron"
}
```

Then, you can start Ferron using:

```sh
docker-compose up -d
```

## Ferron image tags

Ferron provides the following tags for the Ferron image (for Ferron 2):

- `2` - Based on Distroless, statically-linked binaries
- `2-alpine` - Based on Alpine Linux, statically-linked binaries
- `2-debian` - Based on Debian GNU/Linux, dynamically-linked binaries (GNU libc required)
