# shellcheck shell=sh
#
# ui.sh — rendering primitives for the installer UI.
#
# The UI is line-oriented, not screen-oriented. We render by appending lines
# and — in ANSI mode — rewriting only the status cell of the current step
# line. This keeps the implementation small and robust against stray output
# from misbehaving steps (such output just scrolls past; it doesn't corrupt a
# screen buffer we'd otherwise have to maintain).
#
# Status line anatomy:
#
#     [ OK ] Installing binaries
#     ^^^^^^ ^^^^^^^^^^^^^^^^^^^
#       |      label
#       status cell, exactly 6 characters wide (`[`, space, 2- or 4-char
#       state text, space, `]`). Two-letter states like `OK` get an extra
#       leading space so the right bracket stays at the same column as it
#       does for 4-letter states like `FAIL`.
#
# The status cell is rewritten in place by moving the cursor back to column 1,
# erasing the line, and reprinting `[ STATE ] label`.
#
# Globals set by ui_init (in addition to those from tty_init):
#
#   FERRON_UI_C_RESET      SGR reset sequence (or empty).
#   FERRON_UI_C_DIM        Dim/gray SGR.
#   FERRON_UI_C_GREEN      Green SGR.
#   FERRON_UI_C_RED        Red SGR.
#   FERRON_UI_C_YELLOW     Yellow SGR.
#   FERRON_UI_C_BOLD       Bold SGR.
#
#   FERRON_UI_SPINNER_PID  PID of the currently running spinner subshell, or
#                          empty if no spinner is active.
#   FERRON_UI_STEP_LABEL   Label of the currently in-progress step (used to
#                          redraw the line when ending the step).

ui_init() {
    tty_init

    if [ "$FERRON_UI_COLOR" = 1 ]; then
        # Using printf with \033 rather than tput so we don't depend on the
        # full ncurses terminfo database being installed.
        FERRON_UI_C_RESET=$(printf '\033[0m')
        FERRON_UI_C_DIM=$(printf '\033[2m')
        FERRON_UI_C_GREEN=$(printf '\033[32m')
        FERRON_UI_C_RED=$(printf '\033[31m')
        FERRON_UI_C_YELLOW=$(printf '\033[33m')
        FERRON_UI_C_BOLD=$(printf '\033[1m')
    else
        FERRON_UI_C_RESET=''
        FERRON_UI_C_DIM=''
        FERRON_UI_C_GREEN=''
        FERRON_UI_C_RED=''
        FERRON_UI_C_YELLOW=''
        FERRON_UI_C_BOLD=''
    fi

    FERRON_UI_SPINNER_PID=''
    FERRON_UI_STEP_LABEL=''
}

# ui_banner
#
# Print the ASCII-art banner followed by the welcome line. Uses the Unicode
# banner when the locale looks UTF-8 capable, otherwise falls back to the
# plain-ASCII variant.
ui_banner() {
    if [ "$FERRON_UI_UTF8" = 1 ] && \
       [ -r "$FERRON_INSTALLER_EXTRACT_DIR/assets/banner.txt" ]; then
        cat "$FERRON_INSTALLER_EXTRACT_DIR/assets/banner.txt"
    elif [ -r "$FERRON_INSTALLER_EXTRACT_DIR/assets/banner-ascii.txt" ]; then
        cat "$FERRON_INSTALLER_EXTRACT_DIR/assets/banner-ascii.txt"
    fi
    printf '\n'
    printf 'Welcome to the Ferron 3 installer for Linux!\n'
    printf '\n'
}

# ui_info MESSAGE…
#
# Print an informational line between steps (no status cell).
ui_info() {
    printf '%s\n' "$*"
}

# _ui_render_status STATE
#
# Internal: print a 6-character-wide status cell `[ STATE ]`. Two-letter
# states like `OK` are rendered as `[ OK ]` (matching the mockup); four-
# letter states like `FAIL`, `SKIP`, and `WAIT` are rendered as `[FAIL]`
# (without interior spaces) so the outer brackets stay aligned across every
# row. Does NOT emit a trailing newline.
_ui_render_status() {
    case "$1" in
        OK)   color=$FERRON_UI_C_GREEN;  text=' OK ' ;;
        FAIL) color=$FERRON_UI_C_RED;    text='FAIL' ;;
        SKIP) color=$FERRON_UI_C_YELLOW; text='SKIP' ;;
        WAIT) color=$FERRON_UI_C_DIM;    text='....' ;;
        *)    color='';                  text=$(printf '%-4.4s' "$1") ;;
    esac
    printf '[%s%s%s]' "$color" "$text" "$FERRON_UI_C_RESET"
}

# _ui_spinner_frames
#
# Internal: print the spinner frame set as a single space-separated line.
# Braille dots look great on modern terminals; ASCII fallback stays readable
# on serial consoles and minimal busybox shells.
_ui_spinner_frames() {
    if [ "$FERRON_UI_UTF8" = 1 ]; then
        printf '⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏'
    else
        printf '| / - \\'
    fi
}

# _ui_spinner_loop LABEL
#
# Internal: runs in a background subshell. Every ~100ms, rewrites the current
# line with `[ <frame> ] LABEL`. Exits silently when killed by ui_step_end.
_ui_spinner_loop() {
    label=$1
    # shellcheck disable=SC2086  # intentional word-splitting on the frames.
    set -- $(_ui_spinner_frames)
    # Ignore TERM so we can clean up deterministically; the parent kills us.
    trap 'exit 0' TERM INT
    while :; do
        for frame in "$@"; do
            # \r moves to column 1, \033[2K erases the whole line. Together
            # they give us a stable, flicker-free rewrite even when the
            # terminal is narrower than the label. The spinner frame takes
            # the place of the 4-char state text so the cell stays 6 chars
            # wide (`[`, space, frame, 2 spaces, `]`).
            printf '\r\033[2K[ %s%s%s  ] %s' \
                "$FERRON_UI_C_DIM" "$frame" "$FERRON_UI_C_RESET" "$label"
            sleep 0.1 2>/dev/null || sleep 1
        done
    done
}

# ui_step_begin LABEL
#
# Announce the start of a step. In ANSI mode, spawns a spinner that updates
# the status cell until ui_step_end is called. In degraded mode, just prints
# `[ .... ] LABEL` on its own line.
ui_step_begin() {
    FERRON_UI_STEP_LABEL=$1

    if [ "$FERRON_UI_ANSI" = 1 ]; then
        # Hide the cursor so the spinner doesn't make it jitter.
        printf '\033[?25l'
        _ui_spinner_loop "$FERRON_UI_STEP_LABEL" &
        FERRON_UI_SPINNER_PID=$!
    else
        _ui_render_status WAIT
        printf ' %s\n' "$FERRON_UI_STEP_LABEL"
    fi
}

# ui_step_end STATUS
#
# Finalize the currently in-progress step. STATUS is one of OK, FAIL, SKIP.
# In ANSI mode, rewrites the spinner line with the final status and emits a
# newline so subsequent output appears below. In degraded mode, prints a
# fresh line (the earlier `[ .... ]` line stays in the scrollback, which is
# fine for log capture).
ui_step_end() {
    status=$1

    if [ "$FERRON_UI_ANSI" = 1 ]; then
        if [ -n "$FERRON_UI_SPINNER_PID" ]; then
            kill "$FERRON_UI_SPINNER_PID" 2>/dev/null || true
            wait "$FERRON_UI_SPINNER_PID" 2>/dev/null || true
            FERRON_UI_SPINNER_PID=''
        fi
        printf '\r\033[2K'
        _ui_render_status "$status"
        printf ' %s\n' "$FERRON_UI_STEP_LABEL"
        # Restore the cursor.
        printf '\033[?25h'
    else
        _ui_render_status "$status"
        printf ' %s\n' "$FERRON_UI_STEP_LABEL"
    fi

    FERRON_UI_STEP_LABEL=''
}

# ui_spinner_pause
#
# Temporarily kill the spinner and erase its line. Use this before reading
# interactive input so the prompt isn't overwritten by the next spinner
# frame. Pair with ui_spinner_resume.
ui_spinner_pause() {
    if [ "$FERRON_UI_ANSI" = 1 ] && [ -n "$FERRON_UI_SPINNER_PID" ]; then
        kill "$FERRON_UI_SPINNER_PID" 2>/dev/null || true
        wait "$FERRON_UI_SPINNER_PID" 2>/dev/null || true
        FERRON_UI_SPINNER_PID=''
        printf '\r\033[2K'
        printf '\033[?25h'
    fi
}

# ui_spinner_resume
#
# Resume the spinner for the currently-tracked step label. No-op if no step
# is in progress.
ui_spinner_resume() {
    if [ "$FERRON_UI_ANSI" = 1 ] && [ -n "$FERRON_UI_STEP_LABEL" ] && \
       [ -z "$FERRON_UI_SPINNER_PID" ]; then
        printf '\033[?25l'
        _ui_spinner_loop "$FERRON_UI_STEP_LABEL" >&3 &
        FERRON_UI_SPINNER_PID=$!
    fi
}

# ui_success
#
# Final screen for a successful installation. Matches the mockup exactly,
# modulo the emoji which degrades to ":)" in non-UTF-8 locales.
ui_success() {
    if [ "$FERRON_UI_UTF8" = 1 ]; then
        celebrate='🥳'
    else
        celebrate=':)'
    fi
    printf '\n'
    printf '%sFerron is installed successfully!%s %s\n' \
        "$FERRON_UI_C_BOLD" "$FERRON_UI_C_RESET" "$celebrate"
    printf '\n'
    printf 'To access the website, open the web browser and navigate to your server'\''s address.\n'
    printf 'For more information, the documentation is in https://ferron.sh/docs/v3\n'
}

# ui_failure LABEL EXIT_CODE
#
# Final screen for a failed installation. Prints a red header, a tail of the
# log, and a pointer to the full log file. Does NOT exit — the caller decides
# what exit code to propagate.
ui_failure() {
    label=$1
    rc=$2
    printf '\n'
    printf '%sInstallation failed at step:%s %s (exit code %s)\n' \
        "$FERRON_UI_C_RED$FERRON_UI_C_BOLD" "$FERRON_UI_C_RESET" \
        "$label" "$rc"
    printf '\n'
    printf 'Last lines of the installer log:\n'
    log_tail 20
    printf '\n'
    printf 'Full log: %s\n' "$FERRON_INSTALLER_LOG"
}
