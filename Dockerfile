# Use the official Rust image as a build stage
FROM rust as builder

# Set the working directory
WORKDIR /usr/src/ferron

# Copy the source code
COPY . .

# Build the actual application
RUN cargo build --release

# Use a Devuan base image for the final image
FROM devuan/devuan

# Copy the compiled binaries from the builder stage
COPY --from=builder /usr/src/ferron/target/release/ferron /usr/sbin/ferron
COPY --from=builder /usr/src/ferron/target/release/ferron-passwd /usr/sbin/ferron-passwd

# Copy the web server configuration
COPY ferron-docker.yaml /etc/ferron.yaml

# Copy the web root contents
RUN mkdir -p /var/www/ferron
COPY wwwroot/. /var/www/ferron/

# Create a directory where Ferron logs are stored
RUN mkdir -p /var/log/ferron

# Create a "ferron" user and grant the permissions for the log directory and the webroot to that user
RUN useradd -d /nonexistent -s /usr/sbin/nologin -r ferron && chown -hR ferron:ferron /var/www/ferron && chown -hR ferron:ferron /var/log/ferron

# Expose the port 80 (used for HTTP)
EXPOSE 80

# Switch to "ferron" user
USER ferron

# Set the command to run the binary
CMD ["/usr/sbin/ferron", "-c", "/etc/ferron.yaml"]