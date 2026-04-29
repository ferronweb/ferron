# shellcheck shell=sh
#
# 70_selinux.sh — configure SELinux contexts and booleans for Ferron.
#
# This step runs only for archive installs on RHEL/Fedora systems where
# SELinux is enabled. It mirrors the logic from the RPM %post scriptlet:
#
#   - Sets httpd_can_network_connect boolean (for ACME/TLS and reverse proxy)
#   - Adds file contexts for binaries, config, and data directories
#   - Restores contexts with restorecon
#   - Optionally adds QUIC UDP ports (80, 443) if semanage is available
#
# For package installs (Debian/RHEL RPM) this step skips because the
# package manager's %post scriptlet handles it.
#
# For non-SELinux systems this step skips.
#
# For non-RHEL/Fedora systems this step skips.

step_configure_selinux() {
    # Only run for archive installs.
    if [ "$FERRON_INSTALL_METHOD" != "archive" ]; then
        step_skip "package manager handles SELinux configuration"
        return 0
    fi

    # Skip if SELinux is not enabled.
    if ! type selinuxenabled >/dev/null 2>&1; then
        log_write "SELinux tools not found, skipping SELinux configuration"
        return 0
    fi

    if ! selinuxenabled; then
        log_write "SELinux is not enabled, skipping SELinux configuration"
        return 0
    fi

    log_write "SELinux is enabled, configuring contexts and booleans"

    # ------------------------------------------------------------------
    # Set SELinux boolean for network connectivity (ACME, reverse proxy)
    # ------------------------------------------------------------------
    if type setsebool >/dev/null 2>&1; then
        log_write "setting httpd_can_network_connect boolean"
        if ! setsebool -P httpd_can_network_connect on; then
            log_write "warning: failed to set httpd_can_network_connect boolean"
        fi
    else
        log_write "warning: setsebool not available, skipping boolean configuration"
    fi

    # ------------------------------------------------------------------
    # Set file contexts using semanage
    # ------------------------------------------------------------------
    if type semanage >/dev/null 2>&1; then
        log_write "configuring SELinux file contexts"

        # Ferron binary executable
        if ! semanage fcontext -a -t httpd_exec_t "/usr/sbin/ferron" 2>/dev/null; then
            semanage fcontext -m -t httpd_exec_t "/usr/sbin/ferron" 2>/dev/null || true
        fi

        # Configuration file
        if ! semanage fcontext -a -t httpd_config_t "/etc/ferron/ferron.conf" 2>/dev/null; then
            semanage fcontext -m -t httpd_config_t "/etc/ferron/ferron.conf" 2>/dev/null || true
        fi

        # Web root (content)
        if ! semanage fcontext -a -t httpd_sys_content_t "/var/www/ferron(/.*)?" 2>/dev/null; then
            semanage fcontext -m -t httpd_sys_content_t "/var/www/ferron(/.*)?" 2>/dev/null || true
        fi

        # Log directory
        if ! semanage fcontext -a -t httpd_log_t "/var/log/ferron(/.*)?" 2>/dev/null; then
            semanage fcontext -m -t httpd_log_t "/var/log/ferron(/.*)?" 2>/dev/null || true
        fi

        # Runtime data directory
        if ! semanage fcontext -a -t httpd_var_lib_t "/var/lib/ferron(/.*)?" 2>/dev/null; then
            semanage fcontext -m -t httpd_var_lib_t "/var/lib/ferron(/.*)?" 2>/dev/null || true
        fi

        # PID/runtime directory
        if ! semanage fcontext -a -t httpd_var_run_t "/run/ferron(/.*)?" 2>/dev/null; then
            semanage fcontext -m -t httpd_var_run_t "/run/ferron(/.*)?" 2>/dev/null || true
        fi

        log_write "file contexts configured"
    else
        log_write "warning: semanage not available, skipping file context configuration"
    fi

    # ------------------------------------------------------------------
    # Restore contexts with restorecon
    # ------------------------------------------------------------------
    if type restorecon >/dev/null 2>&1; then
        log_write "restoring SELinux contexts"
        if ! restorecon -r /usr/sbin/ferron \
            /usr/sbin/ferron-kdl2ferron \
            /usr/sbin/ferron-passwd \
            /usr/sbin/ferron-precompress \
            /usr/sbin/ferron-serve \
            /etc/ferron/ferron.conf \
            /var/www/ferron \
            /var/log/ferron \
            /var/lib/ferron \
            /run/ferron 2>/dev/null; then
            log_write "warning: restorecon failed"
        fi
    else
        log_write "warning: restorecon not available, skipping context restoration"
    fi

    # ------------------------------------------------------------------
    # Optionally add QUIC UDP ports (80, 443)
    # ------------------------------------------------------------------
    if type semanage >/dev/null 2>&1; then
        log_write "checking QUIC port configuration"

        if ! semanage port -l | grep -q "http_port_t.*udp.*80"; then
            if ! semanage port -a -t http_port_t -p udp 80 2>/dev/null; then
                semanage port -a -t http_port_t -p udp 80 2>/dev/null || true
            fi
        fi

        if ! semanage port -l | grep -q "http_port_t.*udp.*443"; then
            if ! semanage port -a -t http_port_t -p udp 443 2>/dev/null; then
                semanage port -a -t http_port_t -p udp 443 2>/dev/null || true
            fi
        fi
    fi

    log_write "SELinux configuration complete"
}

run_step "Configuring SELinux" step_configure_selinux
