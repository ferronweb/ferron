#!/bin/sh

# Copy the hello.cgi file to the CGI directory
cp /usr/lib/hello-cgi/hello.cgi /usr/lib/cgi-bin/hello.cgi

# Keep the container running
tail -f /dev/null
