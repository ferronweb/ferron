# shellcheck shell=sh
#
# 90_verify.sh — post-install smoke tests.
#
# This step verifies that the Ferron installation is working correctly.
# It performs multiple checks depending on the install method:
#
# For archive installs:
#   - Binary check: ferron --version
#   - Config validation: ferron validate -c /etc/ferron/ferron.conf
#   - Service status: systemctl or init script status
#   - HTTP check: curl http://localhost/
#
# For package installs:
#   - Package presence: dpkg -l ferron3 or rpm -q ferron3
#   - Service status: same as above
#
# The step does NOT abort on failures — it logs them and lets the
# installer complete. The user can then manually investigate.

step_verify() {
    # ------------------------------------------------------------------
    # Binary check
    # ------------------------------------------------------------------
    log_write "verifying ferron binary"
    if [ -x /usr/sbin/ferron ]; then
        log_write "ferron binary exists and is executable"
    else
        log_write "warning: /usr/sbin/ferron does not exist or is not executable"
    fi

    # Try running --version to ensure it works.
    if /usr/sbin/ferron --version >/dev/null 2>&1; then
        _version=$(/usr/sbin/ferron --version 2>/dev/null | head -1)
        log_write "ferron version: $_version"
    else
        log_write "warning: ferron --version failed"
    fi

    # ------------------------------------------------------------------
    # Configuration validation (archive installs only)
    # ------------------------------------------------------------------
    if [ "$FERRON_INSTALL_METHOD" = "archive" ]; then
        if [ -f /etc/ferron/ferron.conf ]; then
            log_write "configuration file exists: /etc/ferron/ferron.conf"
            if /usr/sbin/ferron validate -c /etc/ferron/ferron.conf >/dev/null 2>&1; then
                log_write "configuration validation passed"
            else
                log_write "warning: configuration validation failed"
            fi
        else
            log_write "warning: configuration file not found"
        fi
    fi

    # ------------------------------------------------------------------
    # Service status check
    # ------------------------------------------------------------------
    log_write "checking service status"

    if [ "$FERRON_HAS_SYSTEMD" = 1 ]; then
        if systemctl is-active --quiet ferron 2>/dev/null; then
            log_write "systemd service is active"
        else
            log_write "systemd service is NOT active (may be started manually)"
        fi
    elif [ -f /etc/init.d/ferron ]; then
        if /etc/init.d/ferron status >/dev/null 2>&1; then
            log_write "init script reports service is running"
        else
            log_write "init script reports service is NOT running"
        fi
    else
        log_write "warning: no service manager detected"
    fi

    # ------------------------------------------------------------------
    # Port check
    # ------------------------------------------------------------------
    log_write "checking port 80"
    if command -v ss >/dev/null 2>&1; then
        _port_check=$(ss -tlnp 2>/dev/null | grep -c ':80 ' || echo "0")
        if [ "$_port_check" -gt 0 ]; then
            log_write "port 80 is listening"
        else
            log_write "warning: port 80 is NOT listening"
        fi
    elif command -v netstat >/dev/null 2>&1; then
        _port_check=$(netstat -tlnp 2>/dev/null | grep -c ':80 ' || echo "0")
        if [ "$_port_check" -gt 0 ]; then
            log_write "port 80 is listening"
        else
            log_write "warning: port 80 is NOT listening"
        fi
    else
        log_write "warning: neither ss nor netstat available for port check"
    fi

    # ------------------------------------------------------------------
    # HTTP check (non-blocking)
    # ------------------------------------------------------------------
    log_write "checking HTTP response"
    if command -v curl >/dev/null 2>&1; then
        if curl -s --max-time 5 http://localhost/ >/dev/null 2>&1; then
            log_write "HTTP check passed (curl)"
        else
            log_write "warning: HTTP check failed (curl)"
        fi
    elif command -v wget >/dev/null 2>&1; then
        if wget -q --timeout=5 -O /dev/null http://localhost/ 2>/dev/null; then
            log_write "HTTP check passed (wget)"
        else
            log_write "warning: HTTP check failed (wget)"
        fi
    else
        log_write "warning: neither curl nor wget available for HTTP check"
    fi

    # ------------------------------------------------------------------
    # Package check (package installs only)
    # ------------------------------------------------------------------
    if [ "$FERRON_INSTALL_METHOD" = "debian" ] || [ "$FERRON_INSTALL_METHOD" = "rhel" ]; then
        if [ "$FERRON_INSTALL_METHOD" = "debian" ]; then
            if dpkg -l ferron3 >/dev/null 2>&1; then
                _pkg_version=$(dpkg -s ferron3 2>/dev/null | awk '/^Version:/{print $2}')
                log_write "Debian package ferron3 is installed (version: $_pkg_version)"
            else
                log_write "warning: Debian package ferron3 is NOT installed"
            fi
        elif [ "$FERRON_INSTALL_METHOD" = "rhel" ]; then
            if rpm -q ferron3 >/dev/null 2>&1; then
                _pkg_version=$(rpm -q --queryformat '%{VERSION}' ferron3 2>/dev/null)
                log_write "RPM package ferron3 is installed (version: $_pkg_version)"
            else
                log_write "warning: RPM package ferron3 is NOT installed"
            fi
        fi
    fi

    # ------------------------------------------------------------------
    # Summary
    # ------------------------------------------------------------------
    log_write "=== verification complete ==="
}

run_step "Verifying installation" step_verify
