#!/bin/sh
#
# main.sh — entry point for the self-extracting Ferron installer.
#
# install.sh (produced by the Makefile) extracts every staging file into
# $FERRON_INSTALLER_EXTRACT_DIR and then sources this script. Our job is to:
#
#   1. Load the UI libraries.
#   2. Render the welcome banner.
#   3. Walk the numbered steps/ directory in order, letting run_step manage
#      the spinner / OK / FAIL transitions for each one.
#   4. Render the success screen (or let ui_failure in step.sh handle the
#      unhappy path).
#
# The extraction directory is cleaned up on successful exit only; a failed
# install leaves the log and the extracted scripts behind for inspection.

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

# Clean up the extraction directory only on clean exit. Failed installs fall
# through ui_failure → exit in run_step, which skips this trap's success
# path; we preserve the log and the extracted scripts in that case.
_cleanup_success() {
    rm -rf "$FERRON_INSTALLER_EXTRACT_DIR"
}

# If anything in main.sh itself (outside of run_step) kills the shell with a
# signal, make sure we restore the cursor that ui_step_begin hid.
_cleanup_signal() {
    ui_spinner_pause || true
    printf '\033[?25h' >/dev/tty 2>/dev/null || true
    exit 130
}
trap _cleanup_signal INT TERM HUP

ui_banner

# Walk the numbered step files in lexical order. Each one calls run_step
# internally, so the loop body just sources them.
for _step in "$D"/steps/[0-9]*.sh; do
    [ -r "$_step" ] || continue
    # shellcheck disable=SC1090  # dynamic sourcing by design.
    . "$_step"
done

ui_success
_cleanup_success
