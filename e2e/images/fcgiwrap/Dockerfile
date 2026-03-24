# Use Debian base image
FROM debian:trixie

# Install necessary `fcgiwrap` packages
RUN --mount=type=cache,sharing=private,target=/var/cache/apt \
    --mount=type=cache,sharing=private,target=/var/lib/apt \
    apt update && \
    DEBIAN_FRONTEND=noninteractive apt install -y fcgiwrap && \
    apt clean && \
    rm -rf /var/lib/apt/lists/* && \
    rm -rf /var/cache/apt/*

# Expose port 9000
EXPOSE 9000

# Set the command
CMD [ "/usr/sbin/fcgiwrap", "-f", "-s", "tcp6:[::]:9000" ]
