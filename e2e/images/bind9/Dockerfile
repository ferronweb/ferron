FROM debian:bookworm-slim

# Install BIND 9 and utilities
RUN apt-get update && apt-get install -y --no-install-recommends \
    bind9 \
    bind9-doc \
    bind9-utils \
    dnsutils \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create directories for BIND 9 config and data
RUN mkdir -p /etc/bind/zones /var/lib/bind /var/log/bind

# Ensure proper permissions
RUN chown -R bind:bind /etc/bind /var/lib/bind /var/log/bind && \
    chmod 755 /etc/bind /var/lib/bind /var/log/bind

# Copy the startup script
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# Expose DNS ports (both TCP and UDP on port 53)
EXPOSE 53/tcp 53/udp

# Use the custom entrypoint
ENTRYPOINT ["/entrypoint.sh"]
