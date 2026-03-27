#!/bin/sh

# Copy the hello.cgi file to the CGI directory
cp /usr/lib/hello-cgi/hello.cgi /usr/lib/cgi-bin/hello.cgi

# Make the hello.cgi file executable
chmod a+rx /usr/lib/cgi-bin/hello.cgi
