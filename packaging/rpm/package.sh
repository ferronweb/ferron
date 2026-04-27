#!/bin/bash

# Get Ferron version
FERRON_VERSION_CARGO=$(cat entrypoint/Cargo.toml | grep -E '^version' | sed -E 's|.*"([0-9a-zA-Z.+-]+)"$$|\1|g')
FERRON_VERSION_GIT=$(git tag --sort=-committerdate | head -n 1 | sed s/[^0-9a-zA-Z.+-]//g)
if [ -z "$FERRON_VERSION_CARGO" ]; then
	FERRON_VERSION=$FERRON_VERSION_GIT
else
	FERRON_VERSION=$FERRON_VERSION_CARGO
fi

# Get target triple from argument
TARGET_TRIPLE=$1
TARGET_DIR=$(pwd)/target/$TARGET_TRIPLE
if [ -z "$TARGET_TRIPLE" ]; then
    TARGET_TRIPLE=$(rustc --print host-tuple)
    TARGET_DIR=$(pwd)/target
fi

# Define the directory paths
PROJECT_ROOT_DIR="$(pwd)"
mkdir -p $PROJECT_ROOT_DIR/dist

# Temporarily change the working directory to the script's directory
pushd "$(dirname "$0")"

# Clean up the workspace
rm -rf data/ rpm/ ferron.spec

# Determine the target triple based on the target architecture
RPM_ARCHITECTURE=""
case "$TARGET_TRIPLE" in
    "aarch64-unknown-linux-gnu") RPM_ARCHITECTURE="aarch64" ;;
    "armv7-unknown-linux-gnueabihf") RPM_ARCHITECTURE="armv7hl" ;;
    "x86_64-unknown-linux-gnu") RPM_ARCHITECTURE="x86_64" ;;
    "i686-unknown-linux-gnu") RPM_ARCHITECTURE="i686" ;;
    "powerpc64le-unknown-linux-gnu") RPM_ARCHITECTURE="ppc64le" ;;
    "riscv64gc-unknown-linux-gnu") RPM_ARCHITECTURE="riscv64" ;;
    "s390x-unknown-linux-gnu") RPM_ARCHITECTURE="s390x" ;;
    "")
        echo "No architecture specified." >&2
        exit 1
        ;;
    *)
        echo "Unsupported target triple: $TARGET_TRIPLE" >&2
        exit 1
        ;;
esac

# Determine the version for an RPM package
RPM_VERSION="$(echo $FERRON_VERSION | sed 's/-/~/g')"

# Create the directory, from which the .rpm package is built
RPM_BUILD_DIRECTORY_NAME="ferron3_${RPM_VERSION}_${RPM_ARCHITECTURE}"
mkdir data

# Move the webroot
mkdir -p data/usr/share/ferron
cp -r $PROJECT_ROOT_DIR/wwwroot data/usr/share/ferron/wwwroot

# Delete the configuration and move the binaries
mkdir -p data/usr/sbin
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f ! -name '*.*' -exec cp {} data/usr/sbin \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.exe' -exec cp {} data/usr/sbin \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.dll' -exec cp {} data/usr/sbin \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.dylib' -exec cp {} data/usr/sbin \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.so' -exec cp {} data/usr/sbin \;

# Copy the systemd service
mkdir -p data/usr/lib/systemd/system
cp ferron.service data/usr/lib/systemd/system/

# Copy the Ferron configuration file (not from the archive)
mkdir -p data/etc/ferron
cp $PROJECT_ROOT_DIR/configs/ferron.pkgunix.conf data/etc/ferron/ferron.conf

# Create empty directories
mkdir -p data/var/lib/ferron
mkdir -p data/var/log/ferron
mkdir -p data/run/ferron

# Copy the RPM spec file from the template
cp ferron-template.spec ferron.spec

# Replace the version and architecture in the spec file
sed -Ei "s/^Version:( *).*/Version:\1$RPM_VERSION/" ferron.spec

# Build the RPM package
rpmbuild -bb --build-in-place --define "_topdir $(pwd)/rpm" --target $RPM_ARCHITECTURE ferron.spec

# Move the RPM package to the distribution directory
mv rpm/RPMS/$RPM_ARCHITECTURE/*.rpm $PROJECT_ROOT_DIR/dist

# Remove the rpm directory
rm -rf rpm/

# Pop the working directory
popd
