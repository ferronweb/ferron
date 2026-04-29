# shellcheck shell=sh
#
# log.sh — installer log file management.
#
# Every step's stdout and stderr are redirected into $FERRON_INSTALLER_LOG so
# that the on-screen UI stays clean and a full transcript is available when
# something goes wrong. The log file lives inside the extraction directory so
# it survives a failed install (the trap in main.sh only cleans up on
# success).

log_init() {
    : "${FERRON_INSTALLER_EXTRACT_DIR:?log_init: FERRON_INSTALLER_EXTRACT_DIR is unset}"
    FERRON_INSTALLER_LOG="$FERRON_INSTALLER_EXTRACT_DIR/install.log"
    : >"$FERRON_INSTALLER_LOG"
    export FERRON_INSTALLER_LOG
}

# log_write MESSAGE…
#
# Append a timestamped line to the log file only. Useful for breadcrumbs that
# shouldn't appear on screen (e.g. detected distro, chosen options).
log_write() {
    printf '[%s] %s\n' "$(date '+%Y-%m-%dT%H:%M:%S')" "$*" \
        >>"$FERRON_INSTALLER_LOG"
}

# log_tail [N]
#
# Print the last N lines (default 20) of the log to stdout, each prefixed with
# two spaces so the failure report visually nests under its header.
log_tail() {
    n=${1:-20}
    if [ -s "$FERRON_INSTALLER_LOG" ]; then
        tail -n "$n" "$FERRON_INSTALLER_LOG" | sed 's/^/  /'
    else
        printf '  (log file is empty)\n'
    fi
}
