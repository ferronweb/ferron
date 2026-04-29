# shellcheck shell=sh
#
# 50_config.sh — install ferron.conf and the default wwwroot.
#
# For archive installs this step installs the configuration file and the
# default web root from the extracted archive. It preserves existing files
# to match the `%config(noreplace)` semantics of RPM and the `conffiles`
# mechanism of Debian packages.
#
# For package installs the step skips itself because the package manager
# handles configuration file placement.
#
# Configuration file:
#   /etc/ferron/ferron.conf — only installed if it does not already exist
#
# Web root:
#   /var/www/ferron — only populated if the directory is empty or missing
#
# Ownership:
#   /etc/ferron/ferron.conf — root:root 0644
#   /var/www/ferron/*       — ferron:ferron 0644 (files), 0755 (dirs)

step_install_config() {
    # Package managers handle configuration.
    if [ "$FERRON_INSTALL_METHOD" != "archive" ]; then
        step_skip "package manager handles configuration"
        return 0
    fi

    # ------------------------------------------------------------------
    # Configuration file
    # ------------------------------------------------------------------
    # Only install the configuration file if it does not already exist.
    # This preserves the user's existing configuration across updates,
    # matching the behavior of package managers.
    if [ -f /etc/ferron/ferron.conf ]; then
        log_write "skipping config install: /etc/ferron/ferron.conf already exists"
    else
        # The bundled config comes from the installer bundle (placed by the
        # Makefile's `prepare` step into $FERRON_INSTALLER_EXTRACT_DIR/ferron.conf),
        # not from the downloaded release archive.
        _conf_src="$FERRON_INSTALLER_EXTRACT_DIR/ferron.conf"
        if [ -f "$_conf_src" ]; then
            log_write "installing /etc/ferron/ferron.conf from installer bundle"
            cp "$_conf_src" /etc/ferron/ferron.conf
            chown root:root /etc/ferron/ferron.conf
            chmod 0644 /etc/ferron/ferron.conf
            log_write "installed /etc/ferron/ferron.conf (mode 0644)"
        else
            log_write "warning: bundled ferron.conf not found in installer bundle at $_conf_src"
        fi
    fi

    # ------------------------------------------------------------------
    # Web root
    # ------------------------------------------------------------------
    # Only populate /var/www/ferron if it does not exist or is empty.
    # This preserves the user's existing website files across updates.
    if [ ! -d /var/www/ferron ]; then
        log_write "creating /var/www/ferron"
        mkdir -p /var/www/ferron
    fi

    _wwwroot_src="$FERRON_EXTRACT_DIR/wwwroot"
    if [ -d "$_wwwroot_src" ]; then
        # Check if the directory already has content.
        if [ -z "$(ls -A /var/www/ferron 2>/dev/null)" ]; then
            log_write "populating /var/www/ferron with default files"
            cp -r "$_wwwroot_src"/* /var/www/ferron/
            chown -R ferron:ferron /var/www/ferron
            chmod -R 0755 /var/www/ferron
            log_write "populated /var/www/ferron"
        else
            log_write "skipping wwwroot install: /var/www/ferron already has content"
        fi
    else
        log_write "warning: wwwroot directory not found in archive"
    fi

    # Clean up extraction directory.
    rm -rf "$FERRON_EXTRACT_DIR"
    log_write "cleaned up extraction directory"

    # Clean up backup directory if update succeeded.
    if [ "$FERRON_INSTALL_MODE" = "update" ]; then
        rm -rf "$FERRON_BACKUP_DIR"
        log_write "cleaned up backup directory"
    fi
}

run_step "Installing configuration" step_install_config
