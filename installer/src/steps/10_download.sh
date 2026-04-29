# shellcheck shell=sh
#
# 10_download.sh — fetch the Ferron release tarball for the detected arch.
#
# For archive installs this step either:
#   1. Downloads the latest release from https://dl.ferron.sh/ (or a
#      user-specified version via the FERRON_VERSION environment variable), or
#   2. Validates a locally downloaded archive that the user provides.
#
# For package installs the step skips itself because the package manager
# handles binary distribution via its own repositories.
#
# Downloaded tarball:
#   https://dl.ferron.sh/{version}/ferron-{version}-{triple}.tar.gz
#
# Checksum verification:
#   https://dl.ferron.sh/{version}/SHA256SUMS  (signed with GPG)
#
# Key variables exported:
#   FERRON_VERSION      — detected or user-specified release version
#   FERRON_ARCHIVE_PATH — path to the downloaded or local archive file

step_download() {
    # Package managers handle binary distribution.
    if [ "$FERRON_INSTALL_METHOD" != "archive" ]; then
        step_skip "package manager handles binary distribution"
        return 0
    fi

    # ------------------------------------------------------------------
    # Detect the download source mode.
    # ------------------------------------------------------------------
    # Honor FERRON_ARCHIVE_PATH as an explicit override for local installs.
    # If it's set and points to an existing file, we use local mode.
    # Otherwise we download from the internet.
    if [ -n "${FERRON_ARCHIVE_PATH:-}" ] && [ -f "$FERRON_ARCHIVE_PATH" ]; then
        FERRON_INSTALL_SOURCE="archive"
        log_write "using local archive: $FERRON_ARCHIVE_PATH"
    else
        FERRON_INSTALL_SOURCE="download"
        log_write "install source: download from internet"
    fi

    # ------------------------------------------------------------------
    # Local archive mode
    # ------------------------------------------------------------------
    if [ "$FERRON_INSTALL_SOURCE" = "archive" ]; then
        # The user may have set FERRON_ARCHIVE_PATH via environment, or we
        # ask them for the path.
        if [ ! -f "$FERRON_ARCHIVE_PATH" ]; then
            ui_spinner_pause
            ask_input FERRON_ARCHIVE_PATH \
                "Path to the downloaded Ferron archive"
            ui_spinner_resume
        fi

        # Validate the file exists and is a valid gzip-compressed tar archive.
        if [ ! -f "$FERRON_ARCHIVE_PATH" ]; then
            log_write "error: archive file not found: $FERRON_ARCHIVE_PATH"
            return 1
        fi

        log_write "validating archive $FERRON_ARCHIVE_PATH"
        if ! tar -tzf "$FERRON_ARCHIVE_PATH" >/dev/null 2>&1; then
            log_write "error: $FERRON_ARCHIVE_PATH is not a valid tar.gz archive"
            return 1
        fi
        log_write "archive validation passed"

        # Try to detect the version from the archive filename (e.g. ferron-3.0.0-x86_64-unknown-linux-gnu.tar.gz).
        _archive_basename=$(basename "$FERRON_ARCHIVE_PATH")
        # Extract version: look for pattern -{digits}.{digits}.{digits}-
        if printf '%s' "$_archive_basename" | grep -qE -- '-[0-9]+\.[0-9]+\.[0-9]+-'; then
            FERRON_VERSION=$(printf '%s' "$_archive_basename" | sed -n 's/.*-\([0-9]\+\.[0-9]\+\.[0-9]\+\)-.*/\1/p')
            log_write "detected version from filename: $FERRON_VERSION"
        else
            log_write "warning: could not detect version from archive filename"
            FERRON_VERSION="unknown"
        fi

        return 0
    fi

    # ------------------------------------------------------------------
    # Download mode
    # ------------------------------------------------------------------

    # ------------------------------------------------------------------
    # Choose the download tool.
    # ------------------------------------------------------------------
    _download_cmd=""
    _download_args=""

    if command -v curl >/dev/null 2>&1; then
        _download_cmd="curl"
        _download_args="-fsSL"
    elif command -v wget >/dev/null 2>&1; then
        _download_cmd="wget"
        _download_args="-qO-"
    else
        log_write "error: neither curl nor wget is available"
        log_write "install one of them and retry, or use FERRON_ARCHIVE_PATH for local install"
        return 1
    fi

    # ------------------------------------------------------------------
    # Determine the version to download.
    # ------------------------------------------------------------------
    # Honor FERRON_VERSION env override first. Then check if the user
    # wants the LTS channel. Finally, fetch the latest version from the
    # server.
    if [ -n "${FERRON_VERSION:-}" ]; then
        log_write "using user-specified version: $FERRON_VERSION"
    elif [ "$FERRON_INSTALL_LTS" = "1" ]; then
        log_write "fetching LTS version from dl.ferron.sh"
        FERRON_VERSION=$($_download_cmd $_download_args https://dl.ferron.sh/lts3.ferron 2>/dev/null)
        if [ -z "$FERRON_VERSION" ]; then
            log_write "error: failed to fetch LTS version from dl.ferron.sh"
            return 1
        fi
        log_write "detected LTS version: $FERRON_VERSION"
    else
        log_write "fetching latest version from dl.ferron.sh"
        FERRON_VERSION=$($_download_cmd $_download_args https://dl.ferron.sh/latest3.ferron 2>/dev/null)
        if [ -z "$FERRON_VERSION" ]; then
            log_write "error: failed to fetch latest version from dl.ferron.sh"
            return 1
        fi
        log_write "detected latest version: $FERRON_VERSION"
    fi

    # ------------------------------------------------------------------
    # Construct the download URL.
    # ------------------------------------------------------------------
    # The archive naming convention is:
    #   ferron-{VERSION}-{ARCH}-unknown-{OS}-{LIBC}.tar.gz
    # Examples:
    #   ferron-3.0.0-x86_64-unknown-linux-gnu.tar.gz
    #   ferron-3.0.0-aarch64-unknown-linux-musl.tar.gz
    #   ferron-3.0.0-x86_64-unknown-freebsd.tar.gz
    #   ferron-3.0.0-aarch64-apple-darwin.tar.gz

    _download_url="https://dl.ferron.sh/$FERRON_VERSION/ferron-$FERRON_VERSION-$FERRON_TARGET_TRIPLE.tar.gz"
    log_write "download URL: $_download_url"

    # ------------------------------------------------------------------
    # Create a temporary file for the download.
    # ------------------------------------------------------------------
    FERRON_ARCHIVE_PATH=$(mktemp /tmp/ferron-download.XXXXXX)
    log_write "downloading to $FERRON_ARCHIVE_PATH"

    # Download the archive.
    if ! $_download_cmd $_download_args "$_download_url" > "$FERRON_ARCHIVE_PATH"; then
        log_write "error: failed to download ferron archive"
        rm -f "$FERRON_ARCHIVE_PATH"
        return 1
    fi

    if [ ! -s "$FERRON_ARCHIVE_PATH" ]; then
        log_write "error: downloaded archive is empty"
        rm -f "$FERRON_ARCHIVE_PATH"
        return 1
    fi

    log_write "download complete ($(du -h "$FERRON_ARCHIVE_PATH" | cut -f1))"

    # ------------------------------------------------------------------
    # Verify the checksum.
    # ------------------------------------------------------------------
    log_write "verifying checksum"
    _expected_checksum=$($_download_cmd $_download_args "https://dl.ferron.sh/$FERRON_VERSION/SHA256SUMS" 2>/dev/null | grep "ferron-$FERRON_VERSION-$FERRON_TARGET_TRIPLE.tar.gz" | awk '{print $1}')

    if [ -n "$_expected_checksum" ]; then
        _actual_checksum=$(sha256sum "$FERRON_ARCHIVE_PATH" | awk '{print $1}')
        if [ "$_expected_checksum" != "$_actual_checksum" ]; then
            log_write "error: checksum mismatch"
            log_write "  expected: $_expected_checksum"
            log_write "  actual:   $_actual_checksum"
            rm -f "$FERRON_ARCHIVE_PATH"
            return 1
        fi
        log_write "checksum verification passed"
    else
        log_write "warning: could not fetch checksum from server, skipping verification"
    fi

    log_write "downloaded ferron $FERRON_VERSION for $FERRON_TARGET_TRIPLE"
}

run_step "Downloading Ferron release" step_download
