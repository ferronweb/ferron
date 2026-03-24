# Use the official Rust image as a build stage
FROM rust:alpine as builder

# Set the working directory
WORKDIR /usr/src/hello-cgi

# Copy the source code
COPY . .

# Install a package for compiling software with musl libc
RUN apk add musl-dev

# Build the actual application (cache dependencies with BuildKit)
RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/usr/src/hello-cgi/target \
    cargo build --release && \
    # Copy executables out of the cache
    mkdir .dist && cp target/release/hello-cgi .dist/hello.cgi

# Use alpine as the final stage
FROM alpine:latest

# Create the /usr/lib/cgi-bin directory
WORKDIR /usr/lib/cgi-bin

# Create the directory for the CGI executable
WORKDIR /usr/lib/hello-cgi

# Copy the executable from the build stage
COPY --from=builder /usr/src/hello-cgi/.dist/hello.cgi /usr/lib/hello-cgi/hello.cgi

# Copy the entrypoint script
COPY entrypoint.sh /usr/lib/hello-cgi/

# Make the entrypoint script executable
RUN chmod +x /usr/lib/hello-cgi/entrypoint.sh

# Set the entrypoint
ENTRYPOINT ["/usr/lib/hello-cgi/entrypoint.sh"]
