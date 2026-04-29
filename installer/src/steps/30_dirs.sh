# shellcheck shell=sh
#
# 30_dirs.sh — create the directory layout Ferron expects.
#
# For archive installs this step creates and chowns all directories that the
# package manager would normally handle via its %dir or Files sections.
#
# For package installs the step skips itself because the package postinst
# creates everything.
#
# Directories created:
#   /etc/ferron          — configuration
#   /var/log/ferron      — logs (owned by ferron:ferron, mode 0750)
#   /var/lib/ferron      — runtime data (owned by ferron:ferron, mode 0750)
#   /var/www/ferron      — web root
#   /run/ferron          — PID files, sockets

step_create_dirs() {
    # Package managers handle directory layout.
    if [ "$FERRON_INSTALL_METHOD" != "archive" ]; then
        step_skip "package manager handles directory layout"
        return 0
    fi

    # Create directories (idempotent — mkdir -p is safe to call repeatedly).
    for _dir in /etc/ferron /var/log/ferron /var/lib/ferron /var/www/ferron /run/ferron; do
        if [ ! -d "$_dir" ]; then
            mkdir -p "$_dir"
            log_write "created directory $_dir"
        else
            log_write "directory $_dir already exists"
        fi
    done

    # Apply ownership.
    # Log and data dirs are owned by the ferron user so the server can write
    # logs and PID files without root.
    if id -u ferron >/dev/null 2>&1; then
        chown ferron:ferron /var/log/ferron /var/lib/ferron /run/ferron
        chmod 0750 /var/log/ferron /var/lib/ferron /run/ferron
        log_write "set ownership on /var/log/ferron, /var/lib/ferron and /run/ferron"
    else
        log_write "warning: ferron user not found, skipping chown"
    fi

    # Web root is owned by root so only root (or the installer) can modify it.
    # The ferron user reads from it but never writes.
    chown root:root /var/www/ferron
    chmod 0755 /var/www/ferron
    log_write "set ownership on /var/www/ferron"
}

run_step "Creating directory layout" step_create_dirs
