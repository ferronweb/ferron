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

# Use Go base image for building grpcurl
FROM golang:1-trixie AS builder-grpcurl

# Set the working directory for grpcurl
WORKDIR /usr/src/grpcurl

# Clone the grpcurl Git repository
RUN git clone https://github.com/fullstorydev/grpcurl.git /usr/src/grpcurl

# Build grpcurl
RUN --mount=type=cache,sharing=private,target=/go/pkg/mod \
    # Build grpcurl
    CGO_ENABLED=0 go build -o /usr/local/bin/grpcurl ./cmd/grpcurl

# Use Debian base image
FROM debian:trixie

# Install necessary packages for the test runner
RUN --mount=type=cache,sharing=private,target=/var/cache/apt \
    --mount=type=cache,sharing=private,target=/var/lib/apt \
    apt update && \
    DEBIAN_FRONTEND=noninteractive apt install -y curl brotli zstd bind9-host iproute2 bsdextrautils argon2 netcat-openbsd jq && \
    apt clean && \
    rm -rf /var/lib/apt/lists/* && \
    rm -rf /var/cache/apt/*

# Copy the built websocat
COPY --from=builder-websocat /usr/src/websocat/.dist /usr/local/bin

# Copy the built grpcurl
COPY --from=builder-grpcurl /usr/local/bin/grpcurl /usr/local/bin
