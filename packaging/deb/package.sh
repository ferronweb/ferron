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
rm -rf ferron3_*/ md5sums.tmp

# Determine the target triple based on the target architecture
DEB_ARCHITECTURE=""
case "$TARGET_TRIPLE" in
    "aarch64-unknown-linux-gnu") DEB_ARCHITECTURE="arm64" ;;
    "armv7-unknown-linux-gnueabihf") DEB_ARCHITECTURE="armhf" ;;
    "x86_64-unknown-linux-gnu") DEB_ARCHITECTURE="amd64" ;;
    "i686-unknown-linux-gnu") DEB_ARCHITECTURE="i386" ;;
    "powerpc64le-unknown-linux-gnu") DEB_ARCHITECTURE="ppc64el" ;;
    "riscv64gc-unknown-linux-gnu") DEB_ARCHITECTURE="riscv64" ;;
    "s390x-unknown-linux-gnu") DEB_ARCHITECTURE="s390x" ;;
    *)
        echo "Unsupported target triple: $TARGET_TRIPLE" >&2
        exit 1
        ;;
esac

# Determine the version for a Debian package
DEB_VERSION="$(echo $FERRON_VERSION | sed 's/-/~/g')"

# Create the directory, from which the .deb package is built
DEB_BUILD_DIRECTORY_NAME="ferron3_${DEB_VERSION}_${DEB_ARCHITECTURE}"
mkdir $DEB_BUILD_DIRECTORY_NAME

# Move the webroot
mkdir -p $DEB_BUILD_DIRECTORY_NAME/usr/share/ferron
cp -r $PROJECT_ROOT_DIR/wwwroot $DEB_BUILD_DIRECTORY_NAME/usr/share/ferron/wwwroot

# Delete the configuration and move the binaries
mkdir -p $DEB_BUILD_DIRECTORY_NAME/usr/sbin
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f ! -name '*.*' -exec cp {} $DEB_BUILD_DIRECTORY_NAME/usr/sbin \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.exe' -exec cp {} $DEB_BUILD_DIRECTORY_NAME/usr/sbin \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.dll' -exec cp {} $DEB_BUILD_DIRECTORY_NAME/usr/sbin \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.dylib' -exec cp {} $DEB_BUILD_DIRECTORY_NAME/usr/sbin \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.so' -exec cp {} $DEB_BUILD_DIRECTORY_NAME/usr/sbin \;

# Copy the systemd service
mkdir -p $DEB_BUILD_DIRECTORY_NAME/usr/lib/systemd/system
cp ferron.service $DEB_BUILD_DIRECTORY_NAME/usr/lib/systemd/system/

# Copy the Ferron configuration file (not from the archive)
mkdir -p $DEB_BUILD_DIRECTORY_NAME/etc
mkdir -p $DEB_BUILD_DIRECTORY_NAME/etc/ferron
cp $PROJECT_ROOT_DIR/configs/ferron.pkgunix.conf $DEB_BUILD_DIRECTORY_NAME/etc/ferron/ferron.conf

# Create empty directories
mkdir -p $DEB_BUILD_DIRECTORY_NAME/var/lib/ferron
mkdir -p $DEB_BUILD_DIRECTORY_NAME/var/log/ferron
mkdir -p $DEB_BUILD_DIRECTORY_NAME/run/ferron

# Calculate MD5 checksums
find $DEB_BUILD_DIRECTORY_NAME -type f -exec md5sum {} \; | sed -E 's|([0-9a-fA-F]+) [^/]+/|\1  |g' > md5sums.tmp

# Copy the Debian package control files
cp -r debian $DEB_BUILD_DIRECTORY_NAME/DEBIAN
mv md5sums.tmp $DEB_BUILD_DIRECTORY_NAME/DEBIAN/md5sums

# Replace the version and architecture in the control file
sed -i "s/^Version: .*/Version: $DEB_VERSION/" $DEB_BUILD_DIRECTORY_NAME/DEBIAN/control
sed -i "s/^Architecture: .*/Architecture: $DEB_ARCHITECTURE/" $DEB_BUILD_DIRECTORY_NAME/DEBIAN/control

# Build the Debian package
dpkg-deb --root-owner-group --build $DEB_BUILD_DIRECTORY_NAME "$PROJECT_ROOT_DIR/dist/ferron3_${DEB_VERSION}_${DEB_ARCHITECTURE}.deb"

# Pop the working directory
popd
