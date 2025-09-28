#!/bin/bash

# Change the working directory to the script's directory
cd "$(dirname "$0")"

# Generate certificates
mkdir certs
openssl req -new -newkey rsa:4096 -nodes \
    -keyout certs/server.key -out certs/server.csr \
    -subj "/CN=localhost"
openssl x509 -req -days 3650 -in certs/server.csr -signkey certs/server.key -out certs/server.crt
chmod a+r certs/*

# Start Ferron in the background and capture its PID
FERRON_CONTAINER="$(docker run --rm -d -p 80:80 -p 443:443 \
    --hostname ferron-smoketest \
    -v ./wwwroot:/var/www/test \
    -v ./certs:/etc/certs \
    -v ./ferron-docker.kdl:/etc/ferron.kdl \
    $FERRON_IMAGE)"

# Perform the smoke test
GOT=$(curl -sk https://localhost/test.txt)
EXPECTED=$(cat wwwroot/test.txt)
sleep 5
if [ "$GOT" = "$EXPECTED" ]; then
    echo "Test passed"
    docker kill -s SIGKILL $FERRON_CONTAINER
else
    echo "Test failed" >&2
    docker kill -s SIGKILL $FERRON_CONTAINER
    exit 1
fi
