---
title: Installation via Docker
---

## Prerequisites
Before proceeding with the installation, ensure you have the following:
- A system with Docker installed. If Docker is not installed, follow the official [Docker installation guide](https://docs.docker.com/get-started/get-docker/).
- Internet connectivity to pull the Ferron Docker image.

## Installation steps
### 1. Pull the Ferron Docker image
To download the latest Ferron image from Docker Hub, run the following command:
```sh
docker pull ferronserver/ferron
```

### 2. Run the Ferron container
Once the image is downloaded, start a Ferron container using the following command:
```sh
docker run --name myferron -d -p 80:80 --restart=always ferronserver/ferron
```
This command does the following:
- `--name myferron`: Assigns a name (`myferron`) to the running container.
- `-d`: Runs the container in detached mode.
- `-p 80:80`: Maps port 80 of the container to port 80 on the host machine.
- `--restart=always`: Ensures the container automatically restarts if it stops or if the system reboots.

## Verifying the installation
To confirm that Ferron is running, execute:
```sh
docker ps
```
This should display a running container with the name `myferron`.

To test the web server, open a browser and navigate to `http://localhost`. You should see the default Ferron welcome page.

Alternatively, use `curl`:
```sh
curl http://localhost
```

## File structure

Ferron on Docker has following file structure:

- _/usr/sbin/ferron_ - Ferron web server
- _/usr/sbin/ferron-passwd_ - Ferron user management tool
- _/usr/lib/libferron\_mod\_*.so_ - dynamically linked modules that can be loaded by Ferron
- _/var/log/ferron/access.log_ - Ferron access log in Combined Log Format
- _/var/log/ferron/error.log_ - Ferron error log
- _/var/www/ferron_ - Ferron's web root
- _/etc/ferron.yaml_ - Ferron configuration

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

### Entering the command shell
If you need to manage the Ferron container:
```sh
docker exec -u root -it myferron bash
```