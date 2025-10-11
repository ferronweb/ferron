# Use the official Rust image as a build stage
FROM --platform=$BUILDPLATFORM rust:trixie AS builder

# Define ARGs for target and build platforms
ARG TARGETPLATFORM
ARG BUILDPLATFORM

# Set the working directory
WORKDIR /usr/src/ferron

# Copy the source code
COPY . .

# Install packages for cross-compiling software
RUN --mount=type=cache,sharing=private,target=/var/cache/apt \
    --mount=type=cache,sharing=private,target=/var/lib/apt \
    --mount=type=cache,sharing=private,target=/usr/local/cargo/registry \
    # Install packages for cross-compiling software with musl libc
    if ! [ "$BUILDPLATFORM" = "$TARGETPLATFORM" ]; then \
    case "$TARGETPLATFORM" in \
    "linux/386") dpkg --add-architecture i386 && apt update && DEBIAN_FRONTEND=noninteractive apt install -y musl-dev:i386 ;; \
    "linux/amd64") dpkg --add-architecture amd64 && apt update && DEBIAN_FRONTEND=noninteractive apt install -y musl-dev:amd64 ;; \
    "linux/arm64") dpkg --add-architecture arm64 && apt update && DEBIAN_FRONTEND=noninteractive apt install -y musl-dev:arm64 ;; \
    "linux/arm/v7") dpkg --add-architecture armhf && apt update && DEBIAN_FRONTEND=noninteractive apt install -y musl-dev:armhf ;; \
    "*") echo "Unsupported target platform for cross-compilation: $TARGETPLATFORM" && exit 1 ;; \
    esac \
    else \
    apt update && DEBIAN_FRONTEND=noninteractive apt install -y musl-dev; \
    fi && \
    # Install cmake, bindgen CLI, and required dependencies
    apt install -y cmake clang libclang-dev && \
    cargo install bindgen-cli

# Build the actual application (cache dependencies with BuildKit)
RUN --mount=type=cache,sharing=private,target=/usr/local/cargo/git \
    --mount=type=cache,sharing=private,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/usr/src/ferron/target \
    --mount=type=cache,sharing=private,target=/usr/src/ferron/build-prepare/target \
    # Determine the target
    TARGET_TRIPLE="" && \
    LIB_PATH="" && \
    SYSROOT_PATH="" && \
    if ! [ "$BUILDPLATFORM" = "$TARGETPLATFORM" ]; then \
    case "$TARGETPLATFORM" in \
    "linux/386") TARGET_TRIPLE="i686-unknown-linux-musl" && COMPILER_PREFIX="i686-linux-gnu-" && LIB_PATH="/usr/lib/i386-linux-musl"; ;; \
    "linux/amd64") TARGET_TRIPLE="x86_64-unknown-linux-musl" && COMPILER_PREFIX="x86_64-linux-gnu-" && LIB_PATH="/usr/lib/x86_64-linux-musl"; ;; \
    "linux/arm64") TARGET_TRIPLE="aarch64-unknown-linux-musl" && COMPILER_PREFIX="aarch64-linux-gnu-" && LIB_PATH="/usr/lib/aarch64-linux-musl"; ;; \
    "linux/arm/v7") TARGET_TRIPLE="armv7-unknown-linux-musleabihf" && COMPILER_PREFIX="arm-linux-gnueabihf-" && LIB_PATH="/usr/lib/arm-linux-musleabihf"; ;; \
    "*") echo "Unsupported target platform for cross-compilation: $TARGETPLATFORM" && exit 1 ;; \
    esac \
    else \
    TARGET_TRIPLE="$(rustc --print host-tuple | sed 's/gnu/musl/')" && COMPILER_PREFIX="" && LIB_PATH="/usr/lib/$(gcc -dumpmachine | sed 's/gnu/musl/')"; \
    fi && \
    # Create a GCC wrapper script
    echo "#!/bin/sh\n${COMPILER_PREFIX}gcc \"\$@\" -specs \"${LIB_PATH}/musl-gcc.specs\"" > /tmp/musl-gcc && chmod +x /tmp/musl-gcc && \
    # Install the Rustup target
    rustup target add $TARGET_TRIPLE && \
    # Copy self-contained libunwind
    mkdir -p /tmp/lib && cp $(rustc --print target-libdir --target $TARGET_TRIPLE)/self-contained/libunwind.a /tmp/lib && \
    # Configure Cargo
    echo "[target.$TARGET_TRIPLE]\nlinker = \"/tmp/musl-gcc\"\nrustflags = [\"-Clink-args=-L$LIB_PATH\", \"-Clink-args=-L/tmp/lib\", \"-Clink-self-contained=no\"]" >> /usr/local/cargo/config.toml && \
    TARGET_PATH="target/$TARGET_TRIPLE/release" && \
    # Build Ferron
    RUST_LIBC_UNSTABLE_MUSL_V1_2_3=1 \
    CARGO_FINAL_EXTRA_ARGS="--features ferron/config-docker-auto" \
    TARGET="$TARGET_TRIPLE" \
    CC="/tmp/musl-gcc" \
    make build && \
    # Copy executables out of the cache
    mkdir .dist && cp $TARGET_PATH/ferron $TARGET_PATH/ferron-passwd $TARGET_PATH/ferron-yaml2kdl .dist

# Use a Distroless base image for the final image
# (even though the Rust image is based on Debian 13, the compiled binaries work on an image with Debian 12 base, since they're statically linked)
FROM gcr.io/distroless/static-debian12:nonroot

# Copy the compiled binaries from the builder stage
COPY --from=builder /usr/src/ferron/.dist /usr/sbin

# Switch to "nobody" user to make commands like WORKDIR use the correct owner
USER nobody

# Copy the web server configuration
COPY --chown=nobody ferron-docker.kdl /etc/ferron.kdl

# Copy the web root contents
COPY --chown=nobody wwwroot /var/www/ferron/

# Create an ACME cache directory
WORKDIR /var/cache/ferron-acme

# Create a directory where Ferron logs are stored
WORKDIR /var/log/ferron

# Expose the port 80 (used for HTTP)
EXPOSE 80

# Set the command to run the binary
CMD ["/usr/sbin/ferron", "--config-adapter", "docker-auto"]
