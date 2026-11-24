#!/bin/sh

set -eu

: "${FERRON_INSTALLER_EXTRACT_DIR:?main.sh: FERRON_INSTALLER_EXTRACT_DIR is unset}"

D=$FERRON_INSTALLER_EXTRACT_DIR

. "$D/lib/tty.sh"
. "$D/lib/log.sh"
. "$D/lib/ui.sh"
. "$D/lib/prompt.sh"
. "$D/lib/step.sh"

log_init
ui_init

# Clean up the extraction directory only on clean exit.
_cleanup_success() {
    rm -rf "$FERRON_INSTALLER_EXTRACT_DIR"
}

# Restore the cursor that ui_step_begin hid if anything kills the shell with a signal.
_cleanup_signal() {
    ui_spinner_pause || true
    printf '\033[?25h' >/dev/tty 2>/dev/null || true
    exit 130
}
trap _cleanup_signal INT TERM HUP

ui_banner

# Steps are called via sorted, numbered script files
for _step in "$D"/steps/[0-9]*.sh; do
    [ -r "$_step" ] || continue
    # shellcheck disable=SC1090  # dynamic sourcing by design.
    . "$_step"
done

ui_success
_cleanup_success # Failed installs fall through ui_failure and exit in run_step...
