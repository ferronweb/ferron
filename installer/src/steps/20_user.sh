# shellcheck shell=sh

step_create_user() {
    if [ "$FERRON_INSTALL_METHOD" != "archive" ]; then
        step_skip "package manager handles user creation"
        return 0
    fi

    _has_groupadd=0
    _has_useradd=0
    _has_addgroup=0
    _has_adduser=0

    command -v groupadd >/dev/null 2>&1 && _has_groupadd=1
    command -v useradd >/dev/null 2>&1 && _has_useradd=1
    command -v addgroup >/dev/null 2>&1 && _has_addgroup=1
    command -v adduser >/dev/null 2>&1 && _has_adduser=1
.
    if ! getent group ferron >/dev/null 2>&1; then
        log_write "creating group ferron"
        if [ "$_has_groupadd" = 1 ]; then
            groupadd -r ferron || {
                log_write "groupadd failed, trying addgroup"
                if [ "$_has_addgroup" = 1 ]; then
                    addgroup -S ferron
                else
                    log_write "error: neither groupadd nor addgroup available"
                    return 1
                fi
            }
        elif [ "$_has_addgroup" = 1 ]; then
            addgroup -S ferron || {
                log_write "error: addgroup -S ferron failed"
                return 1
            }
        else
            log_write "error: neither groupadd nor addgroup available"
            return 1
        fi
    else
        log_write "group ferron already exists"
    fi

    if ! id -u ferron >/dev/null 2>&1; then
        log_write "creating user ferron"
        if [ "$_has_useradd" = 1 ]; then
            useradd -r -g ferron -d /var/lib/ferron -m \
                -s /usr/sbin/nologin ferron || {
                log_write "useradd failed, trying adduser"
                if [ "$_has_adduser" = 1 ]; then
                    adduser -S -G ferron -D -H \
                        -h /var/lib/ferron -s /sbin/nologin ferron
                else
                    log_write "error: neither useradd nor adduser available"
                    return 1
                fi
            }
        elif [ "$_has_adduser" = 1 ]; then
            adduser -S -G ferron -D -H \
                -h /var/lib/ferron -s /sbin/nologin ferron || {
                log_write "error: adduser failed"
                return 1
            }
        else
            log_write "error: neither useradd nor adduser available"
            return 1
        fi
        log_write "created user ferron (uid=$(id -u ferron))"
    else
        log_write "user ferron already exists (uid=$(id -u ferron))"

        _user_gid=$(id -g ferron)
        _group_gid=$(getent group ferron | cut -d: -f3)
        if [ "$_user_gid" != "$_group_gid" ]; then
            log_write "updating ferron user's primary group to ferron"
            if [ "$_has_useradd" = 1 ]; then
                usermod -g ferron ferron
            elif [ "$_has_adduser" = 1 ]; then
                log_write "warning: cannot update group with adduser, skipping"
            else
                log_write "warning: cannot update group, no usermod available"
            fi
        fi
    fi
}

if [ "$FERRON_INSTALL_MODE" = "uninstall" ]; then
    : # not applicable in uninstall mode
elif [ "$FERRON_INSTALL_METHOD" = "archive" ]; then
    run_step "Creating ferron system user" step_create_user
fi
