# Use the official Rust image as a build stage
FROM --platform=$BUILDPLATFORM rust:trixie AS builder

# Define ARGs for target and build platforms
ARG TARGETPLATFORM
ARG BUILDPLATFORM

# Install packages for cross-compiling software
RUN --mount=type=cache,sharing=private,target=/var/cache/apt \
    --mount=type=cache,sharing=private,target=/var/lib/apt \
    --mount=type=cache,sharing=private,target=/usr/local/cargo/git \
    --mount=type=cache,sharing=private,target=/usr/local/cargo/registry \
    # Install packages for cross-compiling software
    apt update && DEBIAN_FRONTEND=noninteractive \
    apt install -y debootstrap clang lld libclang-dev wrk nghttp2-client openssl python3 llvm qemu-user-static cmake && \
    cargo install bindgen-cli

# Install the right Rust target
RUN \
    # Determine the target
    TARGET_TRIPLE="" && \
    if ! [ "$BUILDPLATFORM" = "$TARGETPLATFORM" ]; then \
    case "$TARGETPLATFORM" in \
    "linux/386") TARGET_TRIPLE="i686-unknown-linux-musl";; \
    "linux/amd64") TARGET_TRIPLE="x86_64-unknown-linux-musl";; \
    "linux/arm64") TARGET_TRIPLE="aarch64-unknown-linux-musl";; \
    "linux/arm/v7") TARGET_TRIPLE="armv7-unknown-linux-musleabihf";; \
    "*") echo "Unsupported target platform for cross-compilation: $TARGETPLATFORM" && exit 1 ;; \
    esac \
    else \
    TARGET_TRIPLE="$(rustc --print host-tuple | sed 's/gnu/musl/')"; \
    fi && \
    # Install the Rust target
    rustup target add $TARGET_TRIPLE && \
    # Save target triple
    echo "$TARGET_TRIPLE" > /tmp/target_triple

# Set the working directory
WORKDIR /usr/src/ferron

# Copy the source code
COPY . .

# Build the application and copy binaries to an accessible location
RUN --mount=type=cache,sharing=private,target=/usr/local/cargo/git \
    --mount=type=cache,sharing=private,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/usr/src/ferron/target \
    --mount=type=cache,sharing=private,target=/usr/src/ferron/cross-build/sysroots/prepared \
    # Set target triple and path
    TARGET_TRIPLE="$(cat /tmp/target_triple)" && \
    TARGET_PATH="target/$TARGET_TRIPLE/release" && \
    # Prepare the sysroot
    ./cross-build/sysroots/prepare-musl.sh $TARGET_TRIPLE && \
    # Build Ferron binaries
    # Check if PGO would be enabled based on target triple
    if [ "$TARGET_TRIPLE" = "x86_64-unknown-linux-musl" ] \
      || [ "$TARGET_TRIPLE" = "aarch64-unknown-linux-musl" ]; then \
      ./cross-build/build.sh $TARGET_TRIPLE --pgo; \
    else \
      # These targets would fail with PGO, due to missing libprofiler_builtins
      ./cross-build/build.sh $TARGET_TRIPLE; \
    fi && \
    # Copy executables out of the cache
    mkdir .dist && cp $TARGET_PATH/ferron $TARGET_PATH/ferron-fmt $TARGET_PATH/ferron-passwd $TARGET_PATH/ferron-precompress $TARGET_PATH/ferron-kdl2ferron $TARGET_PATH/ferron-serve .dist

# Use a Distroless base image for the final image
FROM gcr.io/distroless/static-debian13:nonroot

# Copy the compiled binaries from the builder stage
COPY --from=builder /usr/src/ferron/.dist /usr/local/bin

# Switch to "nobody" user to make commands like WORKDIR use the correct owner
USER nobody

# Create :
# - an ACME cache directory
# - a directory where Ferron logs are stored
# - a configuration directory
WORKDIR /etc/ferron
WORKDIR /etc/ferron/conf.d
WORKDIR /var/cache/ferron-acme
WORKDIR /var/log/ferron

# Copy the web server configuration
COPY --chown=nobody configs/ferron.docker-entrypoint.conf /etc/ferron/ferron.conf
COPY --chown=nobody configs/ferron.docker.conf /etc/ferron/conf.d/00-default.conf

# Copy the web root contents
COPY --chown=nobody wwwroot /var/www/ferron/

# Expose the port 80 (used for HTTP)
EXPOSE 80

# Set the command to run the binary
CMD ["/usr/local/bin/ferron", "run", "-c", "/etc/ferron/ferron.conf"]
