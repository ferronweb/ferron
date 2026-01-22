# Use the official Rust image as a build stage
FROM rust:alpine AS builder

# Set the working directory
WORKDIR /usr/src/grpcweb-test-proxy

# Copy the source code
COPY . .

# Install required packages
RUN apk add musl-dev protoc

# Build the actual application (cache dependencies with BuildKit)
RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/usr/src/grpcweb-test-proxy/target \
    # Debug build would work, since this is just a test proxy anyway...
    cargo build && \
    # Copy executables out of the cache
    mkdir .dist && cp target/debug/grpcweb-test-proxy .dist/grpcweb-test-proxy

# Use alpine as the final stage
FROM alpine:latest

# Copy the executable from the build stage
COPY --from=builder /usr/src/grpcweb-test-proxy/.dist/grpcweb-test-proxy /usr/local/bin/grpcweb-test-proxy

# Set the entrypoint
ENTRYPOINT ["/usr/local/bin/grpcweb-test-proxy"]
