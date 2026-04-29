# shellcheck shell=sh
#
# prompt.sh — interactive input helpers.
#
# Two public entry points:
#
#   ask_input VARNAME "Question" [default]
#   ask_choice VARNAME "Question" opt1 opt2 …
#
# Both assign the user's answer to the shell variable named by VARNAME. In
# non-interactive mode (stdin is not a TTY, typical of `curl … | sh`) they
# honor an environment-variable override of the same name, then fall back to
# the default (for ask_input) or the first option (for ask_choice). If
# neither is available, they die with a clear message pointing at the env
# variable to set.
#
# Prompts automatically pause and resume the spinner if one is running.

# _prompt_validate_varname NAME
#
# Internal: ensure NAME is a syntactically valid shell variable name before
# using it in `eval`. Aborts the installer on violation; a bad varname here
# is always a programmer bug, not user input.
_prompt_validate_varname() {
    case "$1" in
        ''|*[!A-Za-z0-9_]*|[0-9]*)
            printf 'prompt: invalid variable name: %s\n' "$1" >&2
            exit 2
            ;;
    esac
}

# _prompt_assign VARNAME VALUE
#
# Internal: safely set a shell variable by name. VALUE is single-quoted with
# embedded single quotes escaped so arbitrary user input can't break out.
_prompt_assign() {
    _name=$1
    _value=$2
    # Escape every ' as '\'' so the value survives single-quote wrapping.
    _escaped=$(printf '%s' "$_value" | sed "s/'/'\\\\''/g")
    eval "$_name='$_escaped'"
}

# ask_input VARNAME "Question" [default]
ask_input() {
    _prompt_validate_varname "$1"
    _varname=$1
    _question=$2
    _default=${3:-}

    # Honor env override first — works in both interactive and pipe modes.
    eval "_envval=\${$_varname:-}"
    if [ -n "$_envval" ]; then
        log_write "ask_input $_varname: using env override"
        return 0
    fi

    if [ "$FERRON_UI_STDIN" != 1 ]; then
        if [ -n "$_default" ]; then
            log_write "ask_input $_varname: non-interactive, using default: $_default"
            _prompt_assign "$_varname" "$_default"
            return 0
        fi
        printf 'This installer needs a value for %s but stdin is not a terminal.\n' \
            "$_varname" >&2
        printf 'Set the %s environment variable before running the installer.\n' \
            "$_varname" >&2
        exit 2
    fi

    ui_spinner_pause
    printf '\033[2K\r' >&3
    if [ -n "$_default" ]; then
        printf '%s? %s%s [%s]: ' \
            "$FERRON_UI_C_BOLD" "$FERRON_UI_C_RESET" "$_question" "$_default" >&3
    else
        printf '%s? %s%s: ' \
            "$FERRON_UI_C_BOLD" "$FERRON_UI_C_RESET" "$_question" >&3
    fi
    IFS= read -r _answer || _answer=''
    if [ -z "$_answer" ]; then
        _answer=$_default
    fi
    _prompt_assign "$_varname" "$_answer"
    log_write "ask_input $_varname: '$_answer'"
    ui_spinner_resume
}

# ask_choice VARNAME "Question" opt1 opt2 …
#
# Renders a numbered menu. Accepts either the option number or the exact
# option text as input. In non-interactive mode, the first option is used as
# the default unless an env override is set.
ask_choice() {
    _prompt_validate_varname "$1"
    _varname=$1
    _question=$2
    shift 2

    if [ $# -lt 1 ]; then
        printf 'ask_choice: at least one option is required\n' >&2
        exit 2
    fi

    # Env override: must match one of the options literally.
    eval "_envval=\${$_varname:-}"
    if [ -n "$_envval" ]; then
        for _opt in "$@"; do
            if [ "$_opt" = "$_envval" ]; then
                log_write "ask_choice $_varname: env override '$_envval'"
                return 0
            fi
        done
        printf 'Environment variable %s=%s is not a valid choice.\n' \
            "$_varname" "$_envval" >&2
        printf 'Valid choices:' >&2
        for _opt in "$@"; do
            printf ' %s' "$_opt" >&2
        done
        printf '\n' >&2
        exit 2
    fi

    if [ "$FERRON_UI_STDIN" != 1 ]; then
        log_write "ask_choice $_varname: non-interactive, using default: $1"
        _prompt_assign "$_varname" "$1"
        return 0
    fi

    ui_spinner_pause
    printf '\033[2K\r' >&3
    printf '%s? %s%s\n' \
        "$FERRON_UI_C_BOLD" "$FERRON_UI_C_RESET" "$_question" >&3
    _i=1
    for _opt in "$@"; do
        printf '  %s) %s\n' "$_i" "$_opt" >&3
        _i=$((_i + 1))
    done

    while :; do
        printf '  choice [1]: ' >&3
        IFS= read -r _answer || _answer=''
        [ -z "$_answer" ] && _answer=1

        # Numeric selection.
        case "$_answer" in
            ''|*[!0-9]*) ;;
            *)
                if [ "$_answer" -ge 1 ] && [ "$_answer" -le $# ]; then
                    _i=1
                    for _opt in "$@"; do
                        if [ "$_i" = "$_answer" ]; then
                            _prompt_assign "$_varname" "$_opt"
                            log_write "ask_choice $_varname: '$_opt'"
                            ui_spinner_resume
                            return 0
                        fi
                        _i=$((_i + 1))
                    done
                fi
                ;;
        esac

        # Literal match on the option text.
        for _opt in "$@"; do
            if [ "$_opt" = "$_answer" ]; then
                _prompt_assign "$_varname" "$_opt"
                log_write "ask_choice $_varname: '$_opt'"
                ui_spinner_resume
                return 0
            fi
        done

        printf '  invalid choice, please enter a number between 1 and %s\n' "$#" >&3
    done
}
