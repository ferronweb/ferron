#!/bin/bash

# Define the directory paths
PROJECT_ROOT_DIR="$(pwd)"
TARGET_DIR="$PROJECT_ROOT_DIR/$3"
mkdir -p $PROJECT_ROOT_DIR/dist

# Temporarily change the working directory to the script's directory
pushd "$(dirname "$0")"

# Clean up the workspace
rm -rf ferron_*/ md5sums.tmp

# Determine the target triple based on the target architecture
TARGET_TRIPLE="$1"
DEB_ARCHITECTURE=""
case "$TARGET_TRIPLE" in
    "aarch64-unknown-linux-gnu") DEB_ARCHITECTURE="arm64" ;;
    "armv7-unknown-linux-gnueabihf") DEB_ARCHITECTURE="armhf" ;;
    "x86_64-unknown-linux-gnu") DEB_ARCHITECTURE="amd64" ;;
    "i686-unknown-linux-gnu") DEB_ARCHITECTURE="i386" ;;
    "powerpc64le-unknown-linux-gnu") DEB_ARCHITECTURE="ppc64el" ;;
    "riscv64gc-unknown-linux-gnu") DEB_ARCHITECTURE="riscv64" ;;
    "s390x-unknown-linux-gnu") DEB_ARCHITECTURE="s390x" ;;
    "")
        echo "No architecture specified." >&2
        exit 1
        ;;
    *)
        echo "Unsupported target triple: $TARGET_TRIPLE" >&2
        exit 1
        ;;
esac

# Use the Ferron version from the command line argument
FERRON_VERSION="$2"
if [ "$FERRON_VERSION" = "" ]; then
    echo "No Ferron version specified." >&2
    exit 1
fi

# Create the directory, from which the .deb package is built
DEB_BUILD_DIRECTORY_NAME="ferron_${FERRON_VERSION}_${DEB_ARCHITECTURE}"
mkdir $DEB_BUILD_DIRECTORY_NAME

# Move the webroot
mkdir -p $DEB_BUILD_DIRECTORY_NAME/usr/share/ferron
cp -r $PROJECT_ROOT_DIR/wwwroot $DEB_BUILD_DIRECTORY_NAME/usr/share/ferron/wwwroot

# Delete the configuration and move the binaries
mkdir -p $DEB_BUILD_DIRECTORY_NAME/usr/sbin
find $TARGET_DIR -mindepth 1 -maxdepth 1 -type f ! -name "*.*" -o -name "*.exe" -o -name "*.dll" -o -name "*.dylib" -o -name "*.so" | sed -E "s|(.*)|cp \1 $DEB_BUILD_DIRECTORY_NAME/usr/sbin/|" | bash

# Copy the systemd service
mkdir -p $DEB_BUILD_DIRECTORY_NAME/usr/lib/systemd/system
cp ferron.service $DEB_BUILD_DIRECTORY_NAME/usr/lib/systemd/system/

# Copy the Ferron configuration file (not from the archive)
mkdir -p $DEB_BUILD_DIRECTORY_NAME/etc
cp $PROJECT_ROOT_DIR/ferron-packages.kdl $DEB_BUILD_DIRECTORY_NAME/etc/ferron.kdl

# Create empty directories
mkdir -p $DEB_BUILD_DIRECTORY_NAME/var/lib/ferron
mkdir -p $DEB_BUILD_DIRECTORY_NAME/var/log/ferron

# Calculate MD5 checksums
find $DEB_BUILD_DIRECTORY_NAME -type f -exec md5sum {} \; | sed -E 's|([0-9a-fA-F]+) [^/]+/|\1  |g' > md5sums.tmp

# Copy the Debian package control files
cp -r debian $DEB_BUILD_DIRECTORY_NAME/DEBIAN
mv md5sums.tmp $DEB_BUILD_DIRECTORY_NAME/DEBIAN/md5sums

# Replace the version and architecture in the control file
sed -i "s/^Version: .*/Version: $FERRON_VERSION/" $DEB_BUILD_DIRECTORY_NAME/DEBIAN/control
sed -i "s/^Architecture: .*/Architecture: $DEB_ARCHITECTURE/" $DEB_BUILD_DIRECTORY_NAME/DEBIAN/control

# Build the Debian package
dpkg-deb --root-owner-group --build $DEB_BUILD_DIRECTORY_NAME "$PROJECT_ROOT_DIR/dist/ferron_${FERRON_VERSION}_${DEB_ARCHITECTURE}.deb"

# Pop the working directory
popd
