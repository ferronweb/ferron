# shellcheck shell=sh
#
# tty.sh — terminal capability detection.
#
# Sets the following globals after `tty_init` has been called:
#
#   FERRON_UI_TTY      1 if stdout is connected to a terminal, 0 otherwise.
#   FERRON_UI_STDIN    1 if stdin is connected to a terminal, 0 otherwise.
#   FERRON_UI_ANSI     1 if we should emit ANSI escape sequences (cursor moves,
#                      line erases, spinner redraws), 0 otherwise.
#   FERRON_UI_COLOR    1 if we should emit SGR color codes, 0 otherwise.
#                      Honors the NO_COLOR convention (https://no-color.org).
#   FERRON_UI_UTF8     1 if the locale looks UTF-8 capable, 0 otherwise.
#                      Used to pick between Unicode and ASCII glyphs.
#   FERRON_UI_COLS     Detected terminal width, or 80 as a safe default.
#
# The detection is intentionally conservative: when in doubt we pick the
# degraded (plain-text, no-color) variant so logs piped through `tee` or
# captured by CI systems stay readable.

tty_init() {
    if [ -t 1 ]; then
        FERRON_UI_TTY=1
    else
        FERRON_UI_TTY=0
    fi

    if [ -t 0 ]; then
        FERRON_UI_STDIN=1
    else
        FERRON_UI_STDIN=0
    fi

    # ANSI sequences only make sense on a real terminal, and only when TERM
    # isn't explicitly "dumb".
    if [ "$FERRON_UI_TTY" = 1 ] && [ "${TERM:-dumb}" != "dumb" ]; then
        FERRON_UI_ANSI=1
    else
        FERRON_UI_ANSI=0
    fi

    # Color follows ANSI capability, with NO_COLOR as an explicit opt-out.
    if [ "$FERRON_UI_ANSI" = 1 ] && [ -z "${NO_COLOR:-}" ]; then
        FERRON_UI_COLOR=1
    else
        FERRON_UI_COLOR=0
    fi

    case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
        *UTF-8*|*utf-8*|*UTF8*|*utf8*) FERRON_UI_UTF8=1 ;;
        *)                              FERRON_UI_UTF8=0 ;;
    esac

    # Terminal width. `tput cols` is the most portable, but it isn't always
    # installed; fall back to $COLUMNS, then 80.
    if [ "$FERRON_UI_TTY" = 1 ] && command -v tput >/dev/null 2>&1; then
        FERRON_UI_COLS=$(tput cols 2>/dev/null || echo "${COLUMNS:-80}")
    else
        FERRON_UI_COLS=${COLUMNS:-80}
    fi

    # Save stdout to a temporary file descriptor for use in ask_input.
    exec 3>&1

    export FERRON_UI_TTY FERRON_UI_STDIN FERRON_UI_ANSI \
           FERRON_UI_COLOR FERRON_UI_UTF8 FERRON_UI_COLS
}
