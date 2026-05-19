#!/bin/bash
# BIND 9 entrypoint script for test containers
# Handles config validation, logging setup, and graceful shutdown

set -e

# Log startup
echo "Starting BIND 9 DNS server..."

# Replace placeholders in named.conf.tmpl with real forwarders from /etc/resolv.conf
if [ -f /etc/bind/named.conf.tmpl ]; then
    echo "Configuring named.conf from template..."
    cp /etc/bind/named.conf.tmpl /etc/bind/named.conf

    # Extract nameservers from /etc/resolv.conf and format for named.conf
    NAMESERVERS=$(awk '/^nameserver/ {print $2}' /etc/resolv.conf | sed 's/^/forwarders { /; s/$/; };/')

    # Replace placeholder in named.conf with actual forwarders
    sed -i "s|{{FORWARDERS}}|$NAMESERVERS|g" /etc/bind/named.conf
else
    echo "No named.conf.tmpl found, using existing named.conf if available"
fi

# Ensure log directory is writable
mkdir -p /var/log/bind
touch /var/log/bind/named.log
chown -R bind:bind /var/log/bind

# Also ensure /var/lib/bind and /etc/bind/named.conf are writable
mkdir -p /var/lib/bind
chown -R bind:bind /var/lib/bind
[ -f /etc/bind/named.conf ] && chown bind:bind /etc/bind/named.conf

# Validate BIND 9 configuration if named.conf exists
if [ -f /etc/bind/named.conf ]; then
    echo "Validating BIND 9 configuration..."
    named-checkconf /etc/bind/named.conf || {
        echo "ERROR: BIND 9 configuration validation failed"
        exit 1
    }
fi

# Clean up any stale PID files
rm -f /var/run/named/named.pid

# Start BIND 9 in foreground to allow Docker to manage the process
echo "Starting named daemon..."
exec named -u bind -g -c /etc/bind/named.conf
