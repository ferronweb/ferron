#!/bin/bash

# Get Ferron version
FERRON_VERSION_CARGO=$(cat entrypoint/Cargo.toml | grep -E '^version' | sed -E 's|.*"([0-9a-zA-Z.+-]+)"$$|\1|g')
FERRON_VERSION_GIT=$(git tag --sort=-committerdate | head -n 1 | sed s/[^0-9a-zA-Z.+-]//g)
if [ -z "$FERRON_VERSION_CARGO" ]; then
	FERRON_VERSION=$FERRON_VERSION_GIT
else
	FERRON_VERSION=$FERRON_VERSION_CARGO
fi

echo "Using version: $FERRON_VERSION"

# Get target triple from argument
TARGET_TRIPLE=$1
if [ -z "$TARGET_TRIPLE" ]; then
    TARGET_TRIPLE=$(rustc --print host-tuple)
fi

echo "Target triple: $TARGET_TRIPLE"

# Remove old SBOMs
find . -type f -name '*.cdx.json' -exec rm {} \;
find . -type f -name '*.cdx.xml' -exec rm {} \;

# Invoke cargo cyclonedx
cargo cyclonedx -f json --describe binaries --target "$TARGET_TRIPLE"
cargo cyclonedx -f xml --describe binaries --target "$TARGET_TRIPLE"

# Create a temporary directory for packaging
TEMP_DIR=$(mktemp -d)

# Copy SBOMs to temporary directory
find . -type f -name '*.cdx.json' -exec cp {} $TEMP_DIR \;
find . -type f -name '*.cdx.xml' -exec cp {} $TEMP_DIR \;

# Prepare for packaging
PREVIOUS_DIR=$(pwd)
mkdir -p $PREVIOUS_DIR/dist
FILENAME_NOEXT=$PREVIOUS_DIR/dist/ferron-${FERRON_VERSION}-${TARGET_TRIPLE}-sbom

if echo "$TARGET_TRIPLE" | grep -q 'windows'
then
    # For Windows, create a ZIP archive
    FILENAME=${FILENAME_NOEXT}.zip
    rm -rf $FILENAME
    cd $TEMP_DIR
    zip -r $FILENAME *
    cd -
else
    # For other platforms, create a tar.gz archive
    FILENAME=${FILENAME_NOEXT}.tar.gz
    rm -rf $FILENAME
    cd $TEMP_DIR
    tar -czf $FILENAME *
    cd -
fi

echo "Archive created: $FILENAME"

# Clean up temporary directory
rm -rf $TEMP_DIR
