# shellcheck shell=sh
#
# step.sh — step state machine.
#
# Wraps the "announce step → run command → render final status" lifecycle so
# each individual step file stays tiny. Steps that fail abort the installer
# via ui_failure; steps that succeed fall through to the next one.
#
# Usage:
#
#     my_step() {
#         do_thing_one
#         do_thing_two
#     }
#     run_step "Short human-readable label" my_step
#
# The command and its arguments are executed in the current shell (no
# subshell) so environment changes made by a step — for example, exporting a
# detected distro name — are visible to subsequent steps. Stdout and stderr
# are redirected to the installer log for the duration of the step.
#
# A step that wants to be skipped (e.g. systemd setup on a non-systemd host)
# should call `step_skip "reason"` and return 0. The WAIT/OK/SKIP/FAIL state
# transitions are handled here, not in the step body.

# step_skip REASON
#
# Mark the current step as skipped. Prints REASON into the log for
# traceability and sets a flag that run_step reads after the step returns.
step_skip() {
    FERRON_STEP_SKIP=1
    log_write "step skipped: $*"
}

# run_step LABEL FUNCTION [ARGS…]
#
# Runs FUNCTION with ARGS, showing LABEL on screen. If FUNCTION exits
# non-zero, renders the failure UI and exits the installer with that code.
run_step() {
    _label=$1
    shift

    FERRON_STEP_SKIP=0
    ui_step_begin "$_label"
    log_write "=== step begin: $_label ==="

    # Run the step with stdout+stderr going to the log, but keep the step
    # executing in the current shell so it can export variables. We capture
    # the exit status by wrapping in an if-guard; `set -e` inside the step
    # body still works because functions inherit errexit from the caller.
    _rc=0
    "$@" >>"$FERRON_INSTALLER_LOG" 2>&1 || _rc=$?

    if [ "$_rc" -ne 0 ]; then
        ui_step_end FAIL
        log_write "=== step failed: $_label (rc=$_rc) ==="
        ui_failure "$_label" "$_rc"
        exit "$_rc"
    fi

    if [ "$FERRON_STEP_SKIP" = 1 ]; then
        ui_step_end SKIP
        log_write "=== step skipped: $_label ==="
    else
        ui_step_end OK
        log_write "=== step ok: $_label ==="
    fi
}
