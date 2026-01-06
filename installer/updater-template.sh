# Display functions
print_welcome() {
    echo " 👋 Welcome to the Ferron 2.x updater for GNU/Linux!"
}

print_task() {
    echo -n "    $1"
}

print_task_success() {
    printf "\r"
    echo -n " ✅"
    printf "\n"
}

print_task_fail() {
    printf "\r"
    echo -n " 💥"
    printf "\n"
}

print_error() {
    echo " 😨 Error:"
    echo "$1" | awk '{print "      " $0}'
}

print_update_failed() {
    echo " 😞 Update failed. You might check the error message above."
}

print_update_success() {
    echo " 🥳 Ferron is updated successfully!"
}

print_update_latest() {
    echo " 🥳 Ferron is already up to date!"
}

do_task() {
    print_task "$1"
    shift
    local TASK_OUTPUT_FILE
    local TASK_OUTPUT
    TASK_OUTPUT_FILE=$(mktemp /tmp/ferron-installer.XXXXXX)
    $* > "$TASK_OUTPUT_FILE" 2>&1
    TASK_EXIT_CODE=$?
    TASK_OUTPUT=$(cat "$TASK_OUTPUT_FILE")
    rm "$TASK_OUTPUT_FILE"
    if [ $TASK_EXIT_CODE -eq 0 ]; then
        print_task_success
    else
        print_task_fail
        print_error "$TASK_OUTPUT"
        print_update_failed
        exit 1
    fi
}

read_input() {
   echo -n " ❓ $1: "
   read $2
}

# Tasks
update_package_lists() {
    case "$DISTRO" in
        "debian") DEBIAN_FRONTEND=noninteractive apt update;;
        "rhel") yum -y makecache;;
        "suse") zypper refresh;;
        "arch") pacman --noconfirm -Sy;;
        "alpine") apk update;;
        "freebsd") ASSUME_ALWAYS_YES=yes pkg update;;
        *) true
    esac
}

install_unzip() {
  case "$DISTRO" in
    "debian") DEBIAN_FRONTEND=noninteractive apt install -y unzip;;
    "rhel") yum -y install unzip;;
    "suse") zypper install -y unzip;;
    "arch") pacman --noconfirm -S unzip;;
    "alpine") apk add --no-interactive unzip;;
    "freebsd") ASSUME_ALWAYS_YES=yes pkg install unzip;;
    *) echo "You need to install unzip manually" >&2
  esac
}

install_curl() {
  case "$DISTRO" in
    "debian") DEBIAN_FRONTEND=noninteractive apt install -y curl;;
    "rhel") yum -y install curl;;
    "suse") zypper install -y curl;;
    "arch") pacman --noconfirm -S curl;;
    "alpine") apk add --no-interactive curl;;
    "freebsd") ASSUME_ALWAYS_YES=yes pkg install curl;;
    *) echo "You need to install curl manually" >&2
  esac
}

check_prerequisities() {
    if [ "$(id -u)" != "0" ]; then
      echo 'You need to have root privileges to update Ferron' >&2
      return 1
    fi
    if ! [ -f /usr/sbin/ferron ]; then
      echo 'Ferron isn'"'"'t installed (or it'"'"'s installed without using Ferron installer)!' >&2
      return 1
    fi
    determine_distro
    local UPDATED_LISTS
    if type curl > /dev/null 2>/dev/null; then
        USE_CURL=1
    elif type wget > /dev/null 2>/dev/null; then
        USE_WGET=1
    else
        if [ "$UPDATED_LISTS" != "1" ]; then
          update_package_lists
          UPDATED_LISTS=1
        fi
        install_curl
        if type curl > /dev/null 2>/dev/null; then
            USE_CURL=1
        elif type wget > /dev/null 2>/dev/null; then
            USE_WGET=1
        else
            echo "Neither curl nor wget is installed. You might install one of them using your package manager." >&2
            return 1
        fi
    fi
    if ! type unzip > /dev/null 2>/dev/null; then
        if [ "$UPDATED_LISTS" != "1" ]; then
          update_package_lists
          UPDATED_LISTS=1
        fi
        install_unzip
        if ! type unzip > /dev/null 2>/dev/null; then
          echo "unzip is not installed. You might install unzip using your package manager." >&2
          return 1
        fi
    fi

    if ! [ -f /etc/.ferron-installer.prop ]; then
      echo manual > /etc/.ferron-installer.prop;
    fi

    INSTALLATION_CHANNEL="$(cat /etc/.ferron-installer.prop)"

    return 0
}

download_ferron_zip() {
        # Detect the machine architecture
        local ARCH
        ARCH=$(uname -m)

        # Normalize architecture name
        case "$ARCH" in
            x86_64) ARCH="x86_64" ;;
            i386 | i486 | i586 | i686) ARCH="i686" ;;
            armv7*) ARCH="armv7" ;;
            aarch64) ARCH="aarch64" ;;
            riscv64) ARCH="riscv64gc" ;;
            s390x) ARCH="s390x" ;;
            ppc64le) ARCH="powerpc64le" ;;
            *) echo "Unknown architecture: $ARCH" &>2; return 1 ;;
        esac

        # Detect the operating system
        local OS
        OS=$(uname -s)

        case "$OS" in
            Linux) OS="linux" ;;
            FreeBSD) OS="freebsd" ;;
            *) echo "Unknown OS: $OS" &>2; return 1 ;;
        esac

        # Detect the C library (use musl libc if not GNU libc or if it's too old)
        local LIBC
        GLIBC_VERSION="$(ldd --version 2>&1 | awk '/ldd/{print $NF}')"
        GLIBC_REQUIRED="2.31"
        if [ "$OS" = "linux" ]; then
            if [ "$GLIBC_VERSION" != "" ] && [ "$(printf '%s\n' "$GLIBC_REQUIRED" "$GLIBC_VERSION" | sort -V | head -n1)" = "$GLIBC_REQUIRED" ];  then
                LIBC="gnu"
            else
                LIBC="musl"
            fi
        else
            LIBC=""
        fi

        # Detect the ABI
        local ABI
        if [ "$ARCH" = "armv7" ]; then
            ABI="eabihf"
        else
            ABI=""
        fi

        # Construct the target triple
        local TARGET_TRIPLE
        if [ -n "$LIBC" ]; then
          TARGET_TRIPLE="${ARCH}-unknown-${OS}-${LIBC}${ABI}"
        elif [ -n "$ABI" ]; then
          TARGET_TRIPLE="${ARCH}-unknown-${OS}-${ABI}"
        else
          TARGET_TRIPLE="${ARCH}-unknown-${OS}"
        fi

        local FERRON_DOWNLOAD_COMMAND_AND_PARAMS
        if [ "$USE_WGET" = "1" ]; then
          FERRON_VERSION="$(wget -qO- https://dl.ferron.sh/latest2.ferron)"
          FERRON_DOWNLOAD_COMMAND_AND_PARAMS="wget -O-"
        elif [ "$USE_CURL" = "1" ]; then
          FERRON_VERSION="$(curl -fsL https://dl.ferron.sh/latest2.ferron)"
          FERRON_DOWNLOAD_COMMAND_AND_PARAMS="curl -fsSL"
        fi
        if [ "$FERRON_VERSION" = "" ]; then
          echo 'There was a problem while determining latest Ferron version!' >&2
          return 1
        fi
        FERRON_ZIP_ARCHIVE="$(mktemp /tmp/ferron.XXXXXX)"
        mv "$FERRON_ZIP_ARCHIVE" "$FERRON_ZIP_ARCHIVE.zip"
        FERRON_ZIP_ARCHIVE="$FERRON_ZIP_ARCHIVE.zip"
        if ! $FERRON_DOWNLOAD_COMMAND_AND_PARAMS "https://dl.ferron.sh/$FERRON_VERSION/ferron-$FERRON_VERSION-$TARGET_TRIPLE.zip" > $FERRON_ZIP_ARCHIVE; then
          echo 'There was a problem while downloading latest Ferron version!' >&2
          return 1
        fi
}

invalid_installation_type() {
  echo 'Invalid Ferron installation type.' >&2
  return 1
}

copy_ferron_files() {
    FERRON_EXTRACTION_DIRECTORY="$(mktemp -d /tmp/ferron.XXXXXX)"
    echo $INSTALLATION_CHANNEL > /etc/.ferron-installer.prop || return 1
    if [ "$FERRON_VERSION" != "" ]; then
      echo "$FERRON_VERSION" > /etc/.ferron-installer.version || return 1
    fi
    unzip $FERRON_ZIP_ARCHIVE -d $FERRON_EXTRACTION_DIRECTORY || return 1
    if [ "$INSTALLATION_CHANNEL" != "manual" ]; then
      rm -f $FERRON_ZIP_ARCHIVE  || return 1
    fi
    mv $FERRON_EXTRACTION_DIRECTORY/ferron{,-*} /usr/sbin || return 1
    chown root:root /usr/sbin/ferron{,-*} || return 1
    chmod a+rx /usr/sbin/ferron{,-*} || return 1
    rm -rf $FERRON_EXTRACTION_DIRECTORY || return 1
    return 0
}

fix_selinux_context() {
    restorecon -r /usr/sbin/ferron{,-*} /usr/bin/ferron-updater /etc/ferron.kdl /var/www/ferron /var/log/ferron /var/lib/ferron || return 1
}

stop_ferron() {
    if ! type systemctl > /dev/null 2>&1; then
      /etc/init.d/ferron stop || return 1
    else
      systemctl stop ferron || return 1
    fi
}

restart_ferron() {
    if ! type systemctl > /dev/null 2>&1; then
      /etc/init.d/ferron start || return 1
    else
      systemctl start ferron || return 1
    fi
}

# Welcome message
print_welcome

# Execute tasks
do_task 'Checking prerequisites and installing required packages' check_prerequisities
if [ "$INSTALLATION_CHANNEL" = "stable" ]; then
  do_task 'Downloading Ferron ZIP archive' download_ferron_zip
  FERRON_CURRENT_VERSION="$(cat /etc/.ferron-installer.version)"
  if [ "$FERRON_CURRENT_VERSION" = "$FERRON_VERSION" ]; then
    rm -f $FERRON_ZIP_ARCHIVE
    print_update_latest
    exit 0
  fi
elif [ "$INSTALLATION_CHANNEL" = "manual" ]; then
  read_input 'Path to the Ferron ZIP archive' FERRON_ZIP_ARCHIVE
else
  do_task 'Refusing to install on invalid installation type' invalid_installation_type
fi
do_task 'Stopping Ferron' stop_ferron
do_task 'Copying Ferron files' copy_ferron_files
if type restorecon >/dev/null 2>&1; then
  do_task 'Fixing SELinux context' fix_selinux_context
fi
do_task 'Restarting Ferron' restart_ferron

# Ferron is updated successfully
print_update_success
