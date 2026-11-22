#!/bin/sh

# Prepare hello.cgi to be able to be run by a web server with CGI support.
cp /usr/lib/hello-cgi/hello.cgi /usr/lib/cgi-bin/hello.cgi
chmod a+rx /usr/lib/cgi-bin/hello.cgi
