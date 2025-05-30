# Use the official Rust image as a build stage
FROM rust AS builder

# Set the working directory
WORKDIR /usr/src/ferron

# Copy the source code
COPY . .

# Build the actual application (cache dependencies with BuildKit)
RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/usr/src/ferron/target \
    cargo build --release && \
    # Copy executables out of the cache
    mkdir .dist && cp target/release/ferron target/release/ferron-passwd .dist

# Use a Distroless base image for the final image
FROM gcr.io/distroless/cc-debian12:nonroot

# Copy the compiled binaries from the builder stage
COPY --from=builder /usr/src/ferron/.dist /usr/sbin

# Switch to "nobody" user to make commands like WORKDIR use the correct owner
USER nobody

# Copy the web server configuration
COPY --chown=nobody ferron-docker.yaml /etc/ferron.yaml

# Copy the web root contents
COPY --chown=nobody wwwroot /var/www/ferron/

# Create a directory where Ferron logs are stored
WORKDIR /var/log/ferron

# Expose the port 80 (used for HTTP)
EXPOSE 80

# Set the command to run the binary
CMD ["/usr/sbin/ferron", "-c", "/etc/ferron.yaml"]
