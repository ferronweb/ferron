# shellcheck shell=sh
#
# 00_preflight.sh — host detection, install method selection, and optional
# package-manager installation.
#
# This is the most complex step because it sets up the environment for every
# subsequent step. It runs first and exports variables that control the rest
# of the installer.
#
# Detection performed:
#   - Root / elevated privileges
#   - Distro name and version (from /etc/os-release, /etc/redhat-release, etc.)
#   - CPU architecture normalized to a target-triple component
#   - C library (glibc vs musl) for Linux
#   - Active init system (systemd vs SysV)
#   - Existing Ferron installation (via installer or package manager)
#
# User interaction:
#   - Asks for install method: archive, Debian/Ubuntu APT, RHEL/Fedora DNF, etc.
#   - If an existing installer-managed install is found, offers update / uninstall
#   - For package methods on fresh installs, sets up the repository
#
# Key variables exported:
#   FERRON_DISTRO          — distro name (debian, rhel, alpine, freebsd, unknown)
#   FERRON_DISTRO_VERSION  — distro version or codename
#   FERRON_ARCH            — normalized architecture (x86_64, aarch64, armv7, …)
#   FERRON_LIBC            — libc variant for Linux (gnu, musl, or empty)
#   FERRON_TARGET_TRIPLE   — full triple used in download URLs
#   FERRON_HAS_SYSTEMD     — 1 if systemd is active, 0 otherwise
#   FERRON_INSTALL_METHOD  — archive, debian, rhel, alpine, freebsd
#   FERRON_INSTALL_MODE    — install, update, uninstall
#   FERRON_INSTALL_LTS     — 1 if LTS channel is requested
#   FERRON_ARCHIVE_PATH    — path to downloaded or local archive (archive mode)

step_preflight() {
    # ------------------------------------------------------------------
    # 1. Root check
    # ------------------------------------------------------------------
    if [ "$(id -u)" != "0" ]; then
        log_write "error: this installer must be run as root"
        printf '%sError: the installer must be run as root (or with sudo).%s\n' \
            "$(printf '\033[1m')" "$(printf '\033[0m')" >&2
        return 1
    fi
    log_write "root check passed"

    # ------------------------------------------------------------------
    # 2. Distro detection
    # ------------------------------------------------------------------
    FERRON_DISTRO="unknown"
    FERRON_DISTRO_VERSION=""

    # Try /etc/os-release first (systemd-based distros, modern distros).
    if [ -r /etc/os-release ]; then
        # shellcheck disable=SC1090
        . /etc/os-release
        case "${ID:-}" in
            debian|ubuntu|devuan|mx|pop|elementary|linuxmint)
                FERRON_DISTRO="debian" ;;
            rhel|fedora|centos|rocky|almalinux|amzn|oracle|scientific)
                FERRON_DISTRO="rhel" ;;
            alpine)
                FERRON_DISTRO="alpine" ;;
            arch)
                FERRON_DISTRO="arch" ;;
            freebsd)
                FERRON_DISTRO="freebsd" ;;
        esac
        FERRON_DISTRO_VERSION="${VERSION_ID:-${VERSION_CODENAME:-}}"
    fi

    # Fallback: check /etc/redhat-release.
    if [ "$FERRON_DISTRO" = "unknown" ] && [ -r /etc/redhat-release ]; then
        if grep -qi 'red hat\|rhel\|enterprise' /etc/redhat-release 2>/dev/null; then
            FERRON_DISTRO="rhel"
        elif grep -qi 'fedora' /etc/redhat-release 2>/dev/null; then
            FERRON_DISTRO="fedora"
        elif grep -qi 'centos' /etc/redhat-release 2>/dev/null; then
            FERRON_DISTRO="centos"
        fi
        FERRON_DISTRO_VERSION=$(grep -oE '[0-9]+(\.[0-9]+)*' /etc/redhat-release 2>/dev/null | head -1)
    fi

    # Fallback: check lsb_release.
    if [ "$FERRON_DISTRO" = "unknown" ] && command -v lsb_release >/dev/null 2>&1; then
        _lsb_id=$(lsb_release -si 2>/dev/null | tr '[:upper:]' '[:lower:]')
        case "$_lsb_id" in
            debian|ubuntu|devuan) FERRON_DISTRO="debian" ;;
            redhat|fedora|centos) FERRON_DISTRO="rhel" ;;
            alpine)               FERRON_DISTRO="alpine" ;;
            arch)                 FERRON_DISTRO="arch" ;;
            freebsd)              FERRON_DISTRO="freebsd" ;;
        esac
        _lsb_rel=$(lsb_release -sr 2>/dev/null)
        [ -n "$_lsb_rel" ] && FERRON_DISTRO_VERSION="$_lsb_rel"
    fi

    # Fallback check if arch_release exists.
    if [ "$FERRON_DISTRO" = "unknown" ] && [ -f /etc/arch-release ]; then
        FERRON_DISTRO="arch"
    fi

    # Final fallback: just use the hostname's first word.
    if [ "$FERRON_DISTRO" = "unknown" ]; then
        log_write "warning: could not detect distro, defaulting to unknown"
    else
        log_write "detected distro: $FERRON_DISTRO $FERRON_DISTRO_VERSION"
    fi

    # ------------------------------------------------------------------
    # 3. Architecture detection
    # ------------------------------------------------------------------
    _uname_arch=$(uname -m 2>/dev/null || echo "unknown")
    case "$_uname_arch" in
        x86_64|amd64)    FERRON_ARCH="x86_64" ;;
        aarch64|arm64)    FERRON_ARCH="aarch64" ;;
        armv7l|armv7)     FERRON_ARCH="armv7" ;;
        armv6l)           FERRON_ARCH="armv6" ;;
        riscv64)          FERRON_ARCH="riscv64" ;;
        s390x)            FERRON_ARCH="s390x" ;;
        ppc64le)          FERRON_ARCH="powerpc64le" ;;
        ppc64)            FERRON_ARCH="powerpc64" ;;
        i686|i586|i486|i386) FERRON_ARCH="i686" ;;
        *)                FERRON_ARCH="$_uname_arch" ;;
    esac
    log_write "detected architecture: $FERRON_ARCH (uname: $_uname_arch)"

    # ------------------------------------------------------------------
    # 4. C library detection (Linux only)
    # ------------------------------------------------------------------
    FERRON_LIBC=""
    FERRON_OS=$(uname -s 2>/dev/null || echo "unknown")
    case "$FERRON_OS" in
        Linux)
            # Detect glibc version via ldd.
            _glibc_version=""
            if command -v ldd >/dev/null 2>&1; then
                _glibc_version=$(ldd --version 2>&1 | awk '/ldd/{print $NF}' | head -1)
            fi

            # If glibc >= 2.31, use "gnu"; otherwise musl.
            if [ -n "$_glibc_version" ]; then
                if printf '%s\n' "2.31" "$_glibc_version" | sort -V | head -1 | grep -q '^2\.31$'; then
                    FERRON_LIBC="gnu"
                else
                    FERRON_LIBC="musl"
                fi
            else
                # Try to detect musl explicitly.
                if command -v musl-gcc >/dev/null 2>&1 || \
                   ldd --version 2>&1 | grep -qi musl; then
                    FERRON_LIBC="musl"
                else
                    # Default to glibc if we can't determine.
                    FERRON_LIBC="gnu"
                fi
            fi
            log_write "detected libc: $FERRON_LIBC (ldd version: $_glibc_version)"
            ;;
        FreeBSD)
            # FreeBSD binaries are built without a libc suffix in the triple.
            FERRON_LIBC=""
            ;;
        Darwin)
            # macOS binaries are built without a libc suffix.
            FERRON_LIBC=""
            ;;
    esac

    # ------------------------------------------------------------------
    # 5. Target triple construction
    # ------------------------------------------------------------------
    # The triple format is: ARCH-unknown-OS-LIBC (libc omitted for non-Linux).
    # Examples:
    #   x86_64-unknown-linux-gnu
    #   x86_64-unknown-linux-musl
    #   aarch64-unknown-freebsd
    #   aarch64-apple-darwin
    if [ -n "$FERRON_LIBC" ]; then
        FERRON_TARGET_TRIPLE="${FERRON_ARCH}-unknown-$(echo $FERRON_OS | tr '[:upper:]' '[:lower:]')-${FERRON_LIBC}"
    else
        FERRON_TARGET_TRIPLE="${FERRON_ARCH}-unknown-$(echo $FERRON_OS | tr '[:upper:]' '[:lower:]')"
    fi

    # For FreeBSD, normalize the OS name in the triple.
    case "$FERRON_OS" in
        FreeBSD) FERRON_TARGET_TRIPLE="${FERRON_ARCH}-unknown-freebsd" ;;
        Darwin)  FERRON_TARGET_TRIPLE="${FERRON_ARCH}-apple-darwin" ;;
    esac

    log_write "target triple: $FERRON_TARGET_TRIPLE"

    # ------------------------------------------------------------------
    # 6. Init system detection
    # ------------------------------------------------------------------
    # systemd is "active" if /run/systemd/system exists. This is the same
    # check used by systemd itself to determine whether it is PID 1.
    if [ -d /run/systemd/system ] 2>/dev/null; then
        FERRON_HAS_SYSTEMD=1
        log_write "active init system: systemd"
    else
        FERRON_HAS_SYSTEMD=0
        log_write "active init system: SysV (systemd not active)"
    fi

    # ------------------------------------------------------------------
    # 7. Existing Ferron installation detection
    # ------------------------------------------------------------------
    FERRON_EXISTING_INSTALL=0
    FERRON_EXISTING_METHOD=""

    # Check if installed via this installer.
    if [ -f /etc/.ferron-installer.version ]; then
        FERRON_EXISTING_INSTALL=1
        FERRON_EXISTING_METHOD="installer"
        FERRON_PREVIOUS_VERSION=$(cat /etc/.ferron-installer.version 2>/dev/null || echo "unknown")
        log_write "existing install detected: via installer (version $FERRON_PREVIOUS_VERSION)"
    fi

    # Check if installed via package manager.
    if [ "$FERRON_EXISTING_INSTALL" = 0 ]; then
        if command -v dpkg >/dev/null 2>&1 && dpkg -l ferron3 >/dev/null 2>&1; then
            FERRON_EXISTING_INSTALL=1
            FERRON_EXISTING_METHOD="debian"
            FERRON_PREVIOUS_VERSION=$(dpkg -s ferron3 2>/dev/null | awk '/^Version:/{print $2}')
            log_write "existing install detected: via Debian package (version $FERRON_PREVIOUS_VERSION)"
        elif command -v rpm >/dev/null 2>&1 && rpm -q ferron3 >/dev/null 2>&1; then
            FERRON_EXISTING_INSTALL=1
            FERRON_EXISTING_METHOD="rhel"
            FERRON_PREVIOUS_VERSION=$(rpm -q --queryformat '%{VERSION}' ferron3 2>/dev/null)
            log_write "existing install detected: via RPM package (version $FERRON_PREVIOUS_VERSION)"
        fi
    fi

    # Check if the binary exists but wasn't detected via package manager.
    if [ "$FERRON_EXISTING_INSTALL" = 0 ] && [ -x /usr/sbin/ferron ]; then
        FERRON_EXISTING_INSTALL=1
        FERRON_EXISTING_METHOD="binary"
        FERRON_PREVIOUS_VERSION=$(/usr/sbin/ferron --version 2>/dev/null | head -1 || echo "unknown")
        log_write "existing binary detected: /usr/sbin/ferron"
    fi

    # ------------------------------------------------------------------
    # 8. Install method selection
    # ------------------------------------------------------------------
    if [ "$FERRON_EXISTING_INSTALL" = 1 ]; then
        # Existing install: offer update or uninstall.
        ui_spinner_pause
        if [ "$FERRON_EXISTING_METHOD" = "installer" ]; then
            if ask_choice FERRON_INSTALL_MODE \
                "Ferron is already installed via the installer. What would you like to do?" \
                "update" "uninstall"; then
                log_write "user chose: $FERRON_INSTALL_MODE"
            fi
        elif [ "$FERRON_EXISTING_METHOD" = "debian" ] || [ "$FERRON_EXISTING_METHOD" = "rhel" ]; then
            if ask_choice FERRON_INSTALL_MODE \
                "Ferron is already installed via the package manager. Manage via package manager?" \
                "update" "uninstall" "skip"; then
                log_write "user chose: $FERRON_INSTALL_MODE"
                # If they chose to manage via package manager, set the method.
                if [ "${FERRON_INSTALL_MODE:-}" = "update" ] || \
                   [ "${FERRON_INSTALL_MODE:-}" = "uninstall" ]; then
                    FERRON_INSTALL_METHOD="$FERRON_EXISTING_METHOD"
                    log_write "using existing package method: $FERRON_INSTALL_METHOD"
                fi
            fi
        else
            # Binary install — offer to replace or skip.
            if ask_choice FERRON_INSTALL_MODE \
                "A Ferron binary was detected. What would you like to do?" \
                "update" "skip"; then
                log_write "user chose: $FERRON_INSTALL_MODE"
            fi
        fi
        ui_spinner_resume

        # If the user chose to skip, we still need a method for the remaining
        # steps (e.g., service management). Default to the existing method.
        if [ "${FERRON_INSTALL_MODE:-skip}" = "skip" ]; then
            FERRON_INSTALL_METHOD="skip"
            log_write "user chose to skip; FERRON_INSTALL_METHOD=skip"
            return 0
        fi

        # If they chose update/uninstall for a package-based install, we can
        # short-circuit: set the method and skip the rest of preflight.
        if [ "$FERRON_EXISTING_METHOD" = "debian" ] || \
           [ "$FERRON_EXISTING_METHOD" = "rhel" ]; then
            FERRON_INSTALL_METHOD="$FERRON_EXISTING_METHOD"
            log_write "using existing package method: $FERRON_INSTALL_METHOD"
            return 0
        fi
    fi

    # Fresh install: only ask for method if we haven't already set it.
    if [ -z "${FERRON_INSTALL_METHOD:-}" ]; then
        AVAILABLE_METHODS="archive"
        if [ "$FERRON_DISTRO" = "debian" ] || \
           [ "$FERRON_DISTRO" = "rhel" ]; then
            AVAILABLE_METHODS="$AVAILABLE_METHODS $FERRON_DISTRO"
        fi
        ui_spinner_pause
        if ask_choice FERRON_INSTALL_METHOD \
            "Choose your install method" \
            $AVAILABLE_METHODS; then
            log_write "user chose install method: $FERRON_INSTALL_METHOD"
        fi
        ui_spinner_resume

        FERRON_INSTALL_MODE="install"
    else
        # Existing installer-managed install (update/uninstall chosen).
        if [ "${FERRON_INSTALL_MODE:-}" = "uninstall" ]; then
            # For uninstall, set method so archive-specific steps skip.
            FERRON_INSTALL_METHOD="uninstall"
            log_write "uninstall mode: skipping rest of preflight"
            return 0
        else
            # Update: default to archive method so the archive-specific steps run.
            FERRON_INSTALL_METHOD="archive"
            log_write "update mode: defaulting to archive method for existing $FERRON_EXISTING_METHOD install"
        fi
    fi

    # ------------------------------------------------------------------
    # 9. Channel selection (for archive downloads)
    # ------------------------------------------------------------------
    if [ "$FERRON_INSTALL_METHOD" = "archive" ]; then
        if [ -z "${FERRON_VERSION:-}" ]; then
            # These lines would be uncommented, once the LTS channel is available.

            #ui_spinner_pause
            #if ask_choice FERRON_INSTALL_CHANNEL \
            #    "Which release channel?" \
            #    "stable" "lts"; then
            #    log_write "user chose channel: $FERRON_INSTALL_CHANNEL"
            #fi
            #ui_spinner_resume

            #case "${FERRON_INSTALL_CHANNEL:-stable}" in
            #    lts) FERRON_INSTALL_LTS=1 ;;
            #    *)   FERRON_INSTALL_LTS=0 ;;
            #esac

            FERRON_INSTALL_LTS=0
        else
            log_write "using user-specified version: $FERRON_VERSION"
            FERRON_INSTALL_LTS=0
        fi
    fi

    # ------------------------------------------------------------------
    # 10. Package manager setup (fresh install only)
    # ------------------------------------------------------------------
    if [ "$FERRON_INSTALL_METHOD" = "debian" ]; then
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
        _codename="${FERRON_DISTRO_VERSION:-}"
        if [ -z "$_codename" ] && command -v lsb_release >/dev/null 2>&1; then
            _codename=$(lsb_release -cs 2>/dev/null || echo "")
        fi
        if [ -z "$_codename" ]; then
            _codename="unknown"
            log_write "warning: could not detect distro codename, using 'unknown'"
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

        # After a package install, we're done with preflight.
        return 0

    elif [ "$FERRON_INSTALL_METHOD" = "rhel" ]; then
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

        # After a package install, we're done with preflight.
        return 0
    fi

    # For archive installs, the download step will handle fetching the archive.
    log_write "preflight complete: method=$FERRON_INSTALL_METHOD"
}

run_step "Running preflight checks" step_preflight
