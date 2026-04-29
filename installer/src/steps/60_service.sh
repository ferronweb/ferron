# shellcheck shell=sh
#
# 60_service.sh — install and enable the Ferron service.
#
# For archive installs this step:
#   1. Detects whether systemd or SysV init is the active init system.
#   2. Generates the appropriate unit file or init script inline (as a
#      heredoc) — no external assets are needed.
#   3. Installs it to the correct location.
#   4. Asks the user whether to enable and start the service now.
#   5. Enables and starts the service.
#
# For package installs the step skips itself because the package manager
# handles service setup via its %systemd_unit, %pre, or postinst scripts.

step_install_service() {
    # Package managers handle service setup.
    if [ "$FERRON_INSTALL_METHOD" != "archive" ]; then
        step_skip "package manager handles service setup"
        return 0
    fi

    # ------------------------------------------------------------------
    # Detect whether we should use systemd or SysV init.
    # ------------------------------------------------------------------
    # systemd is "active" if /run/systemd/system exists. This is the same
    # check used by systemd itself to determine whether it is PID 1.
    if [ -d /run/systemd/system ] 2>/dev/null; then
        FERRON_HAS_SYSTEMD=1
        log_write "detected active init system: systemd"
    else
        FERRON_HAS_SYSTEMD=0
        log_write "detected active init system: SysV (systemd not active)"
    fi

    # ------------------------------------------------------------------
    # Ask the user whether to enable and start the service.
    # ------------------------------------------------------------------
    # In non-interactive mode we default to enabling and starting.
    _enable_default="yes"
    if [ "$FERRON_UI_STDIN" != 1 ]; then
        FERRON_ENABLE_SERVICE="$_enable_default"
        log_write "non-interactive mode: defaulting to enable=$FERRON_ENABLE_SERVICE"
    else
        ui_spinner_pause
        if ask_choice FERRON_ENABLE_SERVICE \
            "Enable and start the Ferron service now?" \
            "yes" "no"; then
            log_write "user chose to enable service: $FERRON_ENABLE_SERVICE"
        else
            log_write "user declined to enable service"
        fi
        ui_spinner_resume
    fi

    # ------------------------------------------------------------------
    # Generate and install the service unit / init script.
    # ------------------------------------------------------------------
    if [ "$FERRON_HAS_SYSTEMD" = 1 ]; then
        # ------------------------------------------------------------------
        # Generate systemd unit file inline.
        # ------------------------------------------------------------------
        _unit_content=$(cat <<'UNIT_EOF'
[Unit]
Description=Ferron web server
After=network.target

[Service]
Type=forking
User=ferron
ExecStart=/usr/sbin/ferron daemon -c /etc/ferron/ferron.conf --pid-file /run/ferron/ferron.pid
ExecReload=/bin/kill -HUP $MAINPID
PIDFile=/run/ferron/ferron.pid
Restart=on-failure
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
UNIT_EOF
)

        _unit_dst="/usr/lib/systemd/system/ferron.service"
        log_write "writing systemd unit file to $_unit_dst"
        printf '%s\n' "$_unit_content" > "$_unit_dst"
        chmod 0644 "$_unit_dst"
        log_write "installed systemd unit (mode 0644)"

        # Reload systemd to pick up the new unit.
        log_write "running systemctl daemon-reload"
        if ! systemctl daemon-reload; then
            log_write "warning: systemctl daemon-reload failed"
        fi

        # Enable and optionally start the service.
        if [ "${FERRON_ENABLE_SERVICE:-no}" = "yes" ]; then
            log_write "enabling and starting ferron service"
            if ! systemctl enable --now ferron; then
                log_write "warning: systemctl enable --now ferron failed"
                # Try enable and start separately for better diagnostics.
                if ! systemctl enable ferron; then
                    log_write "warning: systemctl enable ferron failed"
                fi
                if ! systemctl start ferron; then
                    log_write "warning: systemctl start ferron failed"
                fi
            fi
        else
            log_write "skipping service enable/start per user choice"
        fi

    # ------------------------------------------------------------------
    # Generate SysV init script inline.
    # ------------------------------------------------------------------
    else
        _init_content=$(cat <<'INIT_EOF'
#!/bin/sh
### BEGIN INIT INFO
# Provides:          ferron
# Required-Start:    $network $syslog
# Required-Stop:     $network $syslog
# Default-Start:     2 3 4 5
# Default-Stop:      0 1 6
# Description:       Ferron web server
### END INIT INFO

NAME=ferron
DAEMON=/usr/sbin/ferron
PIDFILE=/run/ferron/${NAME}.pid
CONF=/etc/ferron/${NAME}.conf
USER=ferron

case "$1" in
    start)
        setcap 'cap_net_bind_service=+ep' $DAEMON >/dev/null 2>&1 || true
        start-stop-daemon --start --user $USER --exec $DAEMON \
            -- daemon -c $CONF --pid-file $PIDFILE
        ;;
    stop)
        start-stop-daemon --stop --pidfile $PIDFILE --retry 10
        rm -f $PIDFILE
        ;;
    reload)
        if [ -f $PIDFILE ]; then
            kill -HUP $(cat $PIDFILE)
        fi
        ;;
    restart)
        $0 stop
        $0 start
        ;;
    status)
        if [ -f $PIDFILE ] && kill -0 $(cat $PIDFILE) 2>/dev/null; then
            echo "$NAME is running"
            exit 0
        fi
        echo "$NAME is not running"
        exit 1
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|status}"
        exit 1
        ;;
esac
exit 0
INIT_EOF
)

        _init_dst="/etc/init.d/ferron"
        log_write "writing SysV init script to $_init_dst"
        printf '%s\n' "$_init_content" > "$_init_dst"
        chmod 0755 "$_init_dst"
        log_write "installed init script (mode 0755)"

        # Enable the init script per distro.
        case "$FERRON_DISTRO" in
            debian|ubuntu|devuan|mx|pop|elementary|linuxmint)\
                if ! type setcap >/dev/null 2>&1; then
                    log_write "setcap not found, installing via apt"
                    if ! apt-get install -y libcap2-bin; then
                        log_write "warning: apt-get install libcap2-bin failed"
                    fi
                fi
                log_write "enabling init script via update-rc.d"
                if ! update-rc.d ferron defaults; then
                    log_write "warning: update-rc.d ferron defaults failed"
                fi
                ;;
            rhel|fedora|centos|rocky|almalinux|amzn|oracle|scientific|sles|opensuse)
                if ! type setcap >/dev/null 2>&1; then
                    log_write "setcap not found, installing via yum"
                    if ! yum install -y libcap; then
                        log_write "warning: yum install libcap failed"
                    fi
                fi
                log_write "enabling init script via chkconfig"
                if ! chkconfig --add ferron; then
                    log_write "warning: chkconfig --add ferron failed, trying chkconfig --level"
                    chkconfig --level 35 ferron on 2>/dev/null || true
                fi
                ;;
            alpine)
                if ! type setcap >/dev/null 2>&1; then
                    log_write "setcap not found, installing via apk"
                    if ! apk add --no-cache libcap; then
                        log_write "warning: apk add libcap failed"
                    fi
                fi
                log_write "enabling init script via rc-update"
                if ! rc-update add ferron default; then
                    log_write "warning: rc-update add ferron default failed"
                fi
                ;;
            arch)
                if ! type setcap >/dev/null 2>&1; then
                    log_write "setcap not found, installing via pacman"
                    if ! pacman -S --noconfirm libcap; then
                        log_write "warning: pacman -S libcap failed"
                    fi
                fi
                log_write "enabling init script via rc-update"
                if ! rc-update add ferron default; then
                    log_write "warning: rc-update add ferron default failed"
                fi
                ;;
            freebsd)
                # Setcap is Linux-specific, so we don't need it on FreeBSD.
                log_write "enabling init script via rc.d (FreeBSD)"
                if [ ! -d /etc/rc.d ]; then
                    mkdir -p /etc/rc.d
                fi
                if [ ! -f /etc/rc.d/ferron ]; then
                    cp "$_init_dst" /etc/rc.d/ferron
                fi
                chmod 0755 /etc/rc.d/ferron
                log_write "enabled via /etc/rc.d/ferron"
                ;;
            *)
                log_write "warning: unknown distro $FERRON_DISTRO, cannot enable init script automatically"
                log_write "please run the appropriate command to enable the service manually"
                ;;
        esac

        # Start the service if the user opted in.
        if [ "${FERRON_ENABLE_SERVICE:-no}" = "yes" ]; then
            log_write "starting ferron via init script"
            if ! /etc/init.d/ferron start; then
                log_write "warning: /etc/init.d/ferron start failed"
                log_write "you can start the service manually with: /etc/init.d/ferron start"
            fi
        else
            log_write "skipping service start per user choice"
        fi
    fi
}

run_step "Installing service configuration" step_install_service
