# shellcheck shell=sh
#
# 80_uninstall.sh — uninstall Ferron completely.
#
# This step runs only when FERRON_INSTALL_MODE=uninstall. It detects how
# Ferron was installed (installer-managed or package manager), stops and
# disables the service, removes binaries and configuration files, and
# optionally removes the ferron system user.
#
# For installer-managed installs:
#   - Removes binaries from /usr/sbin/
#   - Removes /etc/ferron, /var/log/ferron, /var/www/ferron, /run/ferron
#   - Removes /etc/.ferron-installer.* metadata
#   - Optionally removes the ferron user/group
#
# For package-based installs:
#   - Uses the package manager to purge the package
#   - Removes service configuration
#   - Optionally removes the ferron user/group

step_uninstall() {
    # This step only runs when FERRON_INSTALL_MODE=uninstall.
    if [ "$FERRON_INSTALL_MODE" != "uninstall" ]; then
        step_skip "not an uninstall operation"
        return 0
    fi

    # Detect how Ferron was installed.
    FERRON_WAS_INSTALLED_BY_INSTALLER=0
    FERRON_WAS_INSTALLED_BY_PACKAGE=0

    if [ -f /etc/.ferron-installer.version ]; then
        FERRON_WAS_INSTALLED_BY_INSTALLER=1
        log_write "uninstall: detected installer-managed installation"
    elif dpkg -l ferron3 >/dev/null 2>&1; then
        FERRON_WAS_INSTALLED_BY_PACKAGE=1
        FERRON_INSTALL_METHOD="debian"
        log_write "uninstall: detected Debian package installation"
    elif rpm -q ferron3 >/dev/null 2>&1; then
        FERRON_WAS_INSTALLED_BY_PACKAGE=1
        FERRON_INSTALL_METHOD="rhel"
        log_write "uninstall: detected RPM package installation"
    elif [ -x /usr/sbin/ferron ]; then
        # Binary-only install detected — treat as installer-managed for removal.
        FERRON_WAS_INSTALLED_BY_INSTALLER=1
        log_write "uninstall: detected binary-only installation"
    fi

    # ------------------------------------------------------------------
    # Stop and disable the service
    # ------------------------------------------------------------------
    log_write "stopping Ferron service"

    if [ "$FERRON_HAS_SYSTEMD" = 1 ]; then
        log_write "stopping via systemctl"
        systemctl stop ferron 2>/dev/null || true
        systemctl disable ferron 2>/dev/null || true
        rm -f /usr/lib/systemd/system/ferron.service
        log_write "removed systemd unit file"
        systemctl daemon-reload 2>/dev/null || true

    elif [ -f /etc/init.d/ferron ]; then
        log_write "stopping via init script"
        /etc/init.d/ferron stop 2>/dev/null || true
        case "$FERRON_DISTRO" in
            debian|ubuntu|devuan|mx|pop|elementary|linuxmint)
                log_write "disabling via update-rc.d"
                update-rc.d -f ferron remove 2>/dev/null || true
                ;;
            rhel|fedora|centos|rocky|almalinux|amzn|oracle|scientific|sles|opensuse)
                log_write "disabling via chkconfig"
                chkconfig --del ferron 2>/dev/null || true
                ;;
            alpine)
                log_write "disabling via rc-update"
                rc-update del ferron default 2>/dev/null || true
                ;;
            arch)
                log_write "disabling via rc-update"
                rc-update del ferron default 2>/dev/null || true
                ;;
            freebsd)
                log_write "disabling via /etc/rc.d/"
                rm -f /etc/rc.d/ferron 2>/dev/null || true
                ;;
        esac
        rm -f /etc/init.d/ferron
        log_write "removed init script"
    fi

    # ------------------------------------------------------------------
    # Remove binaries
    # ------------------------------------------------------------------
    if [ "$FERRON_WAS_INSTALLED_BY_INSTALLER" = 1 ]; then
        log_write "removing binaries from /usr/sbin/"
        for _bin in ferron ferron-kdl2ferron ferron-passwd ferron-precompress ferron-serve; do
            rm -f "/usr/sbin/$_bin" 2>/dev/null || true
            log_write "removed /usr/sbin/$_bin"
        done
    elif [ "$FERRON_WAS_INSTALLED_BY_PACKAGE" = 1 ]; then
        log_write "package manager will handle binary removal"
        case "$FERRON_INSTALL_METHOD" in
            debian)
                log_write "purging ferron3 via apt"
                DEBIAN_FRONTEND=noninteractive apt purge -y ferron3 2>/dev/null || true
                ;;
            rhel)
                log_write "purging ferron3 via yum/dnf"
                if command -v dnf >/dev/null 2>&1; then
                    dnf remove -y ferron3 2>/dev/null || true
                else
                    yum remove -y ferron3 2>/dev/null || true
                fi
                ;;
        esac
    fi

    # ------------------------------------------------------------------
    # Remove files (with confirmation)
    # ------------------------------------------------------------------
    if [ "$FERRON_WAS_INSTALLED_BY_INSTALLER" = 1 ]; then
        log_write "removing configuration and data directories"

        # Remove main directories
        rm -rf /etc/ferron
        log_write "removed /etc/ferron"

        rm -rf /var/log/ferron
        log_write "removed /var/log/ferron"

        rm -rf /var/lib/ferron
        log_write "removed /var/lib/ferron"

        rm -rf /var/www/ferron
        log_write "removed /var/www/ferron"

        rm -rf /run/ferron
        log_write "removed /run/ferron"

        # Remove installer metadata
        rm -f /etc/.ferron-installer.version
        rm -f /etc/.ferron-installer.prop
        rm -f /etc/.ferron-installer-channel
        log_write "removed installer metadata"
    fi

    # ------------------------------------------------------------------
    # Optionally remove the ferron system user
    # ------------------------------------------------------------------
    if [ "$FERRON_WAS_INSTALLED_BY_INSTALLER" = 1 ]; then
        ui_spinner_pause
        if ask_choice FERRON_REMOVE_USER \
            "Remove the ferron system user and group?" \
            "yes" "no"; then
            log_write "user chose: $FERRON_REMOVE_USER"
        else
            log_write "user declined to remove ferron user/group"
        fi
        ui_spinner_resume

        if [ "${FERRON_REMOVE_USER:-no}" = "yes" ]; then
            log_write "removing ferron user and group"
            userdel -r ferron 2>/dev/null || true
            groupdel ferron 2>/dev/null || true
            log_write "removed ferron user and group"
        fi
    fi

    log_write "uninstall complete"
}

run_step "Uninstalling Ferron" step_uninstall
