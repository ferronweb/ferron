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
FERRON_CONTAINER="$(docker run -d -p 8443:8443 \
    --hostname ferron-smoketest \
    -v ./wwwroot:/var/www/test \
    -v ./certs:/etc/certs \
    -v ./ferron-docker.kdl:/etc/ferron.kdl \
    $FERRON_IMAGE)"

# Check Ferron logs in background
docker logs -f $FERRON_CONTAINER &
FERRON_LOG_PID=$!

# Wait for Ferron to start
sleep 5

# Perform the smoke test
GOT=$(curl -sk https://localhost:8443/test.txt)
EXPECTED=$(cat wwwroot/test.txt)
if [ "$GOT" = "$EXPECTED" ]; then
    echo "Test passed"
    docker kill -s SIGKILL $FERRON_CONTAINER > /dev/null
    docker container rm $FERRON_CONTAINER > /dev/null
    kill $FERRON_LOG_PID > /dev/null 2>&1 || true
else
    echo "Test failed" >&2
    docker kill -s SIGKILL $FERRON_CONTAINER > /dev/null
    docker container rm $FERRON_CONTAINER > /dev/null
    kill $FERRON_LOG_PID > /dev/null 2>&1 || true
    exit 1
fi
