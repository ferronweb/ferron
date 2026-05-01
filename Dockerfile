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

# Install the right Rust target and configure Cargo
RUN \
    # Determine the target
    ALPINE_PLATFORM="" && \
    TARGET_TRIPLE="" && \
    LIB_PATH="" && \
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
    case "$TARGET_TRIPLE" in \
    "i686-unknown-linux-musl") ALPINE_PLATFORM="x86" ;; \
    "x86_64-unknown-linux-musl") ALPINE_PLATFORM="x86_64" ;; \
    "aarch64-unknown-linux-musl") ALPINE_PLATFORM="aarch64" ;; \
    "armv7-unknown-linux-musleabihf") ALPINE_PLATFORM="armv7" ;; \
    "*") echo "Unsupported target triple: $TARGET_TRIPLE" && exit 1 ;; \
    esac && \
    # Create a GCC wrapper script
    echo "#!/bin/sh\n${COMPILER_PREFIX}gcc \"\$@\" -specs \"${LIB_PATH}/musl-gcc.specs\"" > /tmp/musl-gcc && chmod +x /tmp/musl-gcc && \
    echo "#!/bin/sh\n${COMPILER_PREFIX}g++ \"\$@\" -specs \"${LIB_PATH}/musl-gcc.specs\" -isystem /tmp/sysroot/usr/include -isystem /tmp/sysroot/usr/include/c++/v1" > /tmp/musl-g++ && chmod +x /tmp/musl-g++ && \
    # Install the Rustup target
    rustup target add $TARGET_TRIPLE && \
    # Copy self-contained libunwind
    mkdir -p /tmp/lib && cp $(rustc --print target-libdir --target $TARGET_TRIPLE)/self-contained/libunwind.a /tmp/lib && \
    # Configure Cargo
    echo "[target.$TARGET_TRIPLE]\nlinker = \"/tmp/musl-gcc\"\nrustflags = [\"-Clink-args=-L$LIB_PATH\", \"-Clink-args=-L/tmp/lib\", \"-Clink-args=-lc++abi\", \"-Clink-self-contained=no\"]" >> /usr/local/cargo/config.toml && \
    # Save target triple
    echo "$TARGET_TRIPLE" > /tmp/target_triple && \
    # Get versions of libc++, libc++ headers and musl
    wget -qO- "https://dl-cdn.alpinelinux.org/alpine/v3.23/main/$ALPINE_PLATFORM/APKINDEX.tar.gz" > /tmp/APKINDEX.tar.gz && \
    tar -xzf /tmp/APKINDEX.tar.gz -C /tmp/ && \
    LIBCXX_STATIC_VERSION=$(awk 'BEGIN { RS="\n\n" } /(^|\n)P:libc\+\+-static($|\n)/' /tmp/APKINDEX | sed -n -E 's/^V:(.*)/\1/p') && \
    LIBCXX_DEV_VERSION=$(awk 'BEGIN { RS="\n\n" } /(^|\n)P:libc\+\+-dev($|\n)/' /tmp/APKINDEX | sed -n -E 's/^V:(.*)/\1/p') && \
    MUSL_DEV_VERSION=$(awk 'BEGIN { RS="\n\n" } /(^|\n)P:musl-dev($|\n)/' /tmp/APKINDEX | sed -n -E 's/^V:(.*)/\1/p') && \
    # Obtain libc++ for static linking
    mkdir /tmp/libcxx && \
    wget -qO- "https://dl-cdn.alpinelinux.org/alpine/v3.23/main/$ALPINE_PLATFORM/libc%2B%2B-static-$LIBCXX_STATIC_VERSION.apk" | tar -xz -C /tmp/libcxx && \
    mv /tmp/libcxx/usr/lib/*.a /tmp/lib && rm -rf /tmp/libcxx && \
    # Prepare C++ sysroot
    mkdir -p /tmp/sysroot && \
    # Obtain libc++ headers
    mkdir /tmp/libcxx-dev && mkdir -p /tmp/sysroot/usr/include/c++/v1/ && \
    wget -qO- "https://dl-cdn.alpinelinux.org/alpine/v3.23/main/$ALPINE_PLATFORM/libc%2B%2B-dev-$LIBCXX_DEV_VERSION.apk" | tar -xz -C /tmp/libcxx-dev && \
    mv /tmp/libcxx-dev/usr/include/c++/v1/* /tmp/sysroot/usr/include/c++/v1/ && rm -rf /tmp/libcxx-dev && \
    # Obtain musl headers
    mkdir /tmp/musl-dev && mkdir -p /tmp/sysroot/usr/include/musl/ && \
    wget -qO- "https://dl-cdn.alpinelinux.org/alpine/v3.23/main/$ALPINE_PLATFORM/musl-dev-$MUSL_DEV_VERSION.apk" | tar -xz -C /tmp/musl-dev && \
    mv /tmp/musl-dev/usr/include/* /tmp/sysroot/usr/include/ && rm -rf /tmp/musl-dev && \
    # Symlink GCC toolchain
    mkdir -p /tmp/sysroot/usr/lib && ln -s /usr/lib/gcc /tmp/sysroot/usr/lib/gcc

# Set the working directory
WORKDIR /usr/src/ferron

# Copy the source code
COPY . .

# Build the application and copy binaries to an accessible location
RUN --mount=type=cache,sharing=private,target=/usr/local/cargo/git \
    --mount=type=cache,sharing=private,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/usr/src/ferron/target \
    # Set target triple and path
    TARGET_TRIPLE="$(cat /tmp/target_triple)" && \
    TARGET_PATH="target/$TARGET_TRIPLE/release" && \
    # Build Ferron binaries
    RUST_LIBC_UNSTABLE_MUSL_V1_2_3=1 \
    CC="/tmp/musl-gcc" \
    CXX="clang++" \
    # There has to be "-U TCMALLOC_INTERNAL_METHODS_ONLY", otherwise linking fails
    CXXFLAGS="-U TCMALLOC_INTERNAL_METHODS_ONLY -isystem/tmp/sysroot/usr/include -I/tmp/sysroot/usr/include/c++/v1 -stdlib=libc++ -std=c++17 -nostdinc++ -static --target=$TARGET_TRIPLE" \
    CXXSTDLIB="c++" \
    cargo build --release --target $TARGET_TRIPLE && \
    # Copy executables out of the cache
    mkdir .dist && cp $TARGET_PATH/ferron $TARGET_PATH/ferron-passwd $TARGET_PATH/ferron-precompress $TARGET_PATH/ferron-kdl2ferron $TARGET_PATH/ferron-serve .dist

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
WORKDIR /var/cache/ferron-acme
WORKDIR /var/log/ferron

# Copy the web server configuration
COPY --chown=nobody configs/ferron.docker.conf /etc/ferron/ferron.conf

# Copy the web root contents
COPY --chown=nobody wwwroot /var/www/ferron/

# Expose the port 80 (used for HTTP)
EXPOSE 80

# Set the command to run the binary
CMD ["/usr/local/bin/ferron", "run", "-c", "/etc/ferron/ferron.conf"]
