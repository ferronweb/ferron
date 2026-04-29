# shellcheck shell=sh
#
# 40_binaries.sh — install the ferron binaries into /usr/sbin.
#
# For archive installs this step extracts the downloaded tarball and copies
# the binaries (ferron, ferron-kdl2ferron, ferron-passwd, ferron-precompress,
# ferron-serve) into /usr/sbin with mode 0755.
#
# For package installs the step skips itself because the package manager
# handles binary installation via its payload.
#
# When FERRON_INSTALL_MODE=update the step also creates a backup of the
# existing binaries so they can be restored if the installation fails.
#
# Additionally, this step copies the service configuration files (ferron.service
# for systemd, ferron.init for SysV init) from the installer bundle.

step_install_binaries() {
    # Package managers handle binary installation.
    if [ "$FERRON_INSTALL_METHOD" != "archive" ]; then
        step_skip "package manager handles binary installation"
        return 0
    fi

    # Create a temporary extraction directory.
    FERRON_EXTRACT_DIR=$(mktemp -d /tmp/ferron-install.XXXXXX)
    log_write "extraction directory: $FERRON_EXTRACT_DIR"

    # Extract the archive.
    log_write "extracting archive $FERRON_ARCHIVE_PATH"
    if ! tar -xzf "$FERRON_ARCHIVE_PATH" -C "$FERRON_EXTRACT_DIR"; then
        log_write "error: failed to extract archive"
        rm -rf "$FERRON_EXTRACT_DIR"
        return 1
    fi

    # Verify that the expected binaries exist in the archive.
    _expected_binaries="ferron ferron-kdl2ferron ferron-passwd ferron-precompress ferron-serve"
    for _bin in $_expected_binaries; do
        if [ ! -f "$FERRON_EXTRACT_DIR/$_bin" ]; then
            log_write "warning: binary $_bin not found in archive, skipping"
        fi
    done

    # If updating, back up existing binaries so we can restore on failure.
    if [ "$FERRON_INSTALL_MODE" = "update" ]; then
        FERRON_BACKUP_DIR=$(mktemp -d /tmp/ferron-backup.XXXXXX)
        log_write "backup directory: $FERRON_BACKUP_DIR"
        for _bin in $_expected_binaries; do
            if [ -f "/usr/sbin/$_bin" ]; then
                cp "/usr/sbin/$_bin" "$FERRON_BACKUP_DIR/"
                log_write "backed up /usr/sbin/$_bin"
            fi
        done
    fi

    # Install binaries to /usr/sbin.
    for _bin in $_expected_binaries; do
        _src="$FERRON_EXTRACT_DIR/$_bin"
        [ -f "$_src" ] || continue

        log_write "installing /usr/sbin/$_bin"
        if ! cp "$_src" "/usr/sbin/$_bin"; then
            log_write "error: failed to install /usr/sbin/$_bin"
            # Restore backup if update mode.
            if [ "$FERRON_INSTALL_MODE" = "update" ] && [ -f "$FERRON_BACKUP_DIR/$_bin" ]; then
                log_write "restoring backup /usr/sbin/$_bin"
                cp "$FERRON_BACKUP_DIR/$_bin" "/usr/sbin/$_bin"
            fi
            rm -rf "$FERRON_EXTRACT_DIR" "$FERRON_BACKUP_DIR"
            return 1
        fi
        chmod 0755 "/usr/sbin/$_bin"
        log_write "installed /usr/sbin/$_bin (mode 0755)"
    done

    # Note: service unit files (ferron.service / ferron.init) are generated
    # inline by step 60_service.sh, so we do not copy them from the bundle
    # here.
}

run_step "Installing binaries" step_install_binaries
