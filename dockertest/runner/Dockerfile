# Use Rust base image for building websocat
FROM rust:trixie AS builder-websocat

# Set the working directory for websocat
WORKDIR /usr/src/websocat

# Clone the websocat Git repository
RUN git clone https://github.com/vi/websocat.git /usr/src/websocat

# Build websocat
RUN --mount=type=cache,sharing=private,target=/usr/local/cargo/git \
    --mount=type=cache,sharing=private,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/usr/src/websocat/target \
    # Build websocat
    cargo build --release && \
    # Copy executables out of the cache
    mkdir .dist && cp target/release/websocat .dist

# Use Debian base image
FROM debian:trixie

# Install necessary packages for the test runner
RUN --mount=type=cache,sharing=private,target=/var/cache/apt \
    --mount=type=cache,sharing=private,target=/var/lib/apt \
    apt update && \
    DEBIAN_FRONTEND=noninteractive apt install -y curl brotli zstd bind9-host iproute2 bsdextrautils argon2 netcat-openbsd && \
    apt clean && \
    rm -rf /var/lib/apt/lists/* && \
    rm -rf /var/cache/apt/*

# Copy the built websocat
COPY --from=builder-websocat /usr/src/websocat/.dist /usr/local/bin
