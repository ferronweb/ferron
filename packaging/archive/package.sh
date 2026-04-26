#!/bin/bash

# Get Ferron version
FERRON_VERSION_CARGO=$(cat ferron/Cargo.toml | grep -E '^version' | sed -E 's|.*"([0-9a-zA-Z.+-]+)"$$|\1|g')
FERRON_VERSION_GIT=$(git tag --sort=-committerdate | head -n 1 | sed s/[^0-9a-zA-Z.+-]//g)
if [ -z "$FERRON_VERSION_CARGO" ]; then
	FERRON_VERSION=$FERRON_VERSION_GIT
else
	FERRON_VERSION=$FERRON_VERSION_CARGO
fi

echo "Using version: $FERRON_VERSION"

# Get target triple from argument
TARGET_TRIPLE=$1
TARGET_DIR=target/$TARGET_TRIPLE
if [ -z "$TARGET_TRIPLE" ]; then
    TARGET_TRIPLE=$(rustc --print host-tuple)
    TARGET_DIR=target
fi

echo "Target triple: $TARGET_TRIPLE"
echo "Target dir: $TARGET_DIR"

# Create a temporary directory for packaging
TEMP_DIR=$(mktemp -d)

# Copy release files to temporary directory
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f ! -name '*.*' -exec cp {} $TEMP_DIR \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.exe' -exec cp {} $TEMP_DIR \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.dll' -exec cp {} $TEMP_DIR \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.dylib' -exec cp {} $TEMP_DIR \;
find $TARGET_DIR/release -mindepth 1 -maxdepth 1 -type f -name '*.so' -exec cp {} $TEMP_DIR \;
cp configs/ferron.release.conf $TEMP_DIR/ferron.conf
cp -r wwwroot $TEMP_DIR/wwwroot

# Prepare for packaging
PREVIOUS_DIR=$(pwd)
mkdir -p $PREVIOUS_DIR/dist
FILENAME_NOEXT=$PREVIOUS_DIR/dist/ferron-${FERRON_VERSION}-${TARGET_TRIPLE}

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
