#!/bin/bash

# Define the directory paths
PROJECT_ROOT_DIR="$(pwd)"
TARGET_DIR="$PROJECT_ROOT_DIR/$3"
mkdir -p $PROJECT_ROOT_DIR/dist

# Temporarily change the working directory to the script's directory
pushd "$(dirname "$0")"

# Clean up the workspace
rm -rf data/ rpm/ ferron.spec

# Determine the target triple based on the target architecture
TARGET_TRIPLE="$1"
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

# Use the Ferron version from the command line argument
FERRON_VERSION="$2"
if [ "$FERRON_VERSION" = "" ]; then
    echo "No Ferron version specified." >&2
    exit 1
fi

# Determine the version for a Debian package
RPM_VERSION="$(echo $FERRON_VERSION | sed 's/-/~/g')"

# Create the directory, from which the .deb package is built
RPM_BUILD_DIRECTORY_NAME="ferron_${RPM_VERSION}_${RPM_ARCHITECTURE}"
mkdir data

# Move the webroot
mkdir -p data/usr/share/ferron
cp -r $PROJECT_ROOT_DIR/wwwroot data/usr/share/ferron/wwwroot

# Delete the configuration and move the binaries
mkdir -p data/usr/sbin
find $TARGET_DIR -mindepth 1 -maxdepth 1 -type f ! -name "*.*" -o -name "*.exe" -o -name "*.dll" -o -name "*.dylib" -o -name "*.so" | sed -E "s|(.*)|cp \1 data/usr/sbin/|" | bash

# Copy the systemd service
mkdir -p data/usr/lib/systemd/system
cp ferron.service data/usr/lib/systemd/system/

# Copy the Ferron configuration file (not from the archive)
mkdir -p data/etc
cp $PROJECT_ROOT_DIR/ferron-packages.kdl data/etc/ferron.kdl

# Create empty directories
mkdir -p data/var/lib/ferron
mkdir -p data/var/log/ferron

# Copy the RPM spec file from the template
cp ferron-template.spec ferron.spec

# Replace the version and architecture in the spec file
sed -Ei "s/^Version:( *).*/Version:\1$RPM_VERSION/" ferron.spec
sed -Ei "s/^BuildArch:( *).*/BuildArch:\1$RPM_ARCHITECTURE/" ferron.spec

# Build the RPM package
rpmbuild -ba --build-in-place --define "_topdir $(pwd)/rpm" ferron.spec

# Move the RPM package to the distribution directory
mv rpm/RPMS/$RPM_ARCHITECTURE/*.rpm $PROJECT_ROOT_DIR/dist

# Remove the rpm directory
rm -rf rpm/

# Pop the working directory
popd
