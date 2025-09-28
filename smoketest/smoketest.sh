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
$FERRON &
FERRON_PID=$!

# Wait for Ferron to start
sleep 5

# Perform the smoke test
GOT=$(curl -sk https://localhost:8443/test.txt)
EXPECTED=$(cat wwwroot/test.txt)
if [ "$GOT" = "$EXPECTED" ]; then
    echo "Test passed"
    kill -9 $FERRON_PID
else
    echo "Test failed" >&2
    kill -9 $FERRON_PID
    exit 1
fi
