# shellcheck shell=sh
#
# 15_package_install.sh — install Ferron via the system package manager.
#
# This step runs when FERRON_INSTALL_METHOD is "debian" or "rhel". It sets up
# the repository (if not already configured) and installs the ferron3 package.
#
# For archive installs, this step is not shown.
# For uninstall mode, this step is not shown (handled by 80_uninstall.sh).

step_package_install() {
    case "$FERRON_INSTALL_METHOD" in
        debian)
            log_write "setting up Debian/Ubuntu repository"

            # Install prerequisites if needed.
            _need_prereqs=0
            for _cmd in curl gnupg2 ca-certificates lsb-release; do
                if ! command -v "$_cmd" >/dev/null 2>&1; then
                    _need_prereqs=1
                    break
                fi
            done
            # debian-archive-keyring might already be present.
            if [ -z "$(dpkg -l debian-archive-keyring 2>/dev/null | grep '^ii')" ]; then
                _need_prereqs=1
            fi

            if [ "$_need_prereqs" = 1 ]; then
                log_write "installing prerequisites: curl gnupg2 ca-certificates lsb-release debian-archive-keyring"

                # Update package lists.
                if ! DEBIAN_FRONTEND=noninteractive apt update; then
                    log_write "warning: apt update failed"
                fi

                if ! DEBIAN_FRONTEND=noninteractive apt install -y \
                        curl gnupg2 ca-certificates lsb-release debian-archive-keyring; then
                    log_write "warning: failed to install prerequisites"
                fi
            fi

            # Install the signing key.
            _keyring="/usr/share/keyrings/ferron-keyring.gpg"
            if [ ! -f "$_keyring" ]; then
                log_write "installing Ferron GPG key"
                if curl -fsSL https://deb.ferron.sh/signing.pgp | \
                       gpg --dearmor -o "$_keyring" 2>/dev/null; then
                    chmod 0644 "$_keyring"
                    log_write "installed GPG key to $_keyring"
                else
                    log_write "warning: failed to install GPG key"
                fi
            fi

            # Add the repository if not already present.
            _sources_list="/etc/apt/sources.list.d/ferron.list"
            _codename="${FERRON_DISTRO_CODENAME:-}"
            if [ -z "$_codename" ] && command -v lsb_release >/dev/null 2>&1; then
                _codename=$(lsb_release -cs 2>/dev/null || echo "")
            fi
            if [ -z "$_codename" ]; then
                _codename="sid"
                log_write "warning: could not detect distro codename, using 'sid'"
            fi

            if [ ! -f "$_sources_list" ] || ! grep -q "deb.ferron.sh" "$_sources_list" 2>/dev/null; then
                log_write "adding repository for codename $_codename"
                printf 'deb [signed-by=%s] https://deb.ferron.sh %s main\n' \
                    "$_keyring" "$_codename" > "$_sources_list"
                log_write "added repository to $_sources_list"
            else
                log_write "repository already configured"
            fi

            # Update package lists.
            log_write "running apt update"
            if ! DEBIAN_FRONTEND=noninteractive apt update; then
                log_write "warning: apt update failed"
            fi

            # Install Ferron.
            log_write "installing ferron3 package"
            if ! DEBIAN_FRONTEND=noninteractive apt install -y ferron3; then
                log_write "error: failed to install ferron3 package"
                return 1
            fi
            log_write "installed ferron3 via APT"
            ;;

        rhel)
            log_write "setting up RHEL/Fedora repository"

            # Install yum-utils if needed.
            if ! command -v yum-config-manager >/dev/null 2>&1 && \
               ! command -v dnf-config-manager >/dev/null 2>&1; then
                log_write "installing yum-utils"
                if command -v dnf >/dev/null 2>&1; then
                    dnf install -y yum-utils 2>/dev/null || true
                else
                    yum install -y yum-utils 2>/dev/null || true
                fi
            fi

            # Add the repository.
            _repo_file="/etc/yum.repos.d/ferron.repo"
            if [ ! -f "$_repo_file" ]; then
                log_write "adding repository from https://rpm.ferron.sh/ferron.repo"
                if command -v yum-config-manager >/dev/null 2>&1; then
                    yum-config-manager --add-repo https://rpm.ferron.sh/ferron.repo 2>/dev/null || true
                elif command -v dnf-config-manager >/dev/null 2>&1; then
                    dnf-config-manager --add-repo https://rpm.ferron.sh/ferron.repo 2>/dev/null || true
                else
                    # Fallback: create the repo file manually.
                    cat > "$_repo_file" <<'REPOEOF'
[ferron]
name=Ferron Repository
baseurl=https://rpm.ferron.sh/ferron.repo
enabled=1
gpgcheck=0
REPOEOF
                    log_write "created repo file $_repo_file (manual)"
                fi
                log_write "added repository to $_repo_file"
            else
                log_write "repository already configured"
            fi

            # Install Ferron.
            log_write "installing ferron3 package"
            if command -v dnf >/dev/null 2>&1; then
                dnf install -y ferron3 2>/dev/null || yum install -y ferron3 2>/dev/null || (log_write "error: failed to install ferron3 via YUM/DNF" && return 1)
            else
                yum install -y ferron3 2>/dev/null || (log_write "error: failed to install ferron3 via YUM/DNF" && return 1)
            fi
            log_write "installed ferron3 via YUM/DNF"
            ;;

        *)
            # Should not reach here due to conditional at bottom.
            step_skip "not a package install method"
            return 0
            ;;
    esac
}

if [ "$FERRON_INSTALL_MODE" = "uninstall" ]; then
    : # uninstall handled by step 80
elif [ "$FERRON_INSTALL_METHOD" = "debian" ] || [ "$FERRON_INSTALL_METHOD" = "rhel" ]; then
    run_step "Installing Ferron package" step_package_install
fi
