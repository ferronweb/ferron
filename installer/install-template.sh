#!/bin/bash

# Display functions
print_welcome() {
    echo " 👋 Welcome to the Ferron 2.x installer for GNU/Linux!"
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

print_installation_failed() {
    echo " 😞 Installation failed. You might check the error message above."
}

print_installation_success() {
    echo " 🥳 Ferron is installed successfully! You can now access your newly-installed web server."
}

do_task() {
    print_task "$1"
    shift
    local TASK_OUTPUT_FILE
    local TASK_OUTPUT
    TASK_OUTPUT_FILE=$(mktemp /tmp/ferron-installer.XXXXXX)
    "$@" > "$TASK_OUTPUT_FILE" 2>&1
    TASK_EXIT_CODE=$?
    TASK_OUTPUT=$(cat "$TASK_OUTPUT_FILE")
    rm "$TASK_OUTPUT_FILE"
    if [ $TASK_EXIT_CODE -eq 0 ]; then
        print_task_success
    else
        print_task_fail
        print_error "$TASK_OUTPUT"
        print_installation_failed
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

install_setcap() {
  case "$DISTRO" in
    "debian") DEBIAN_FRONTEND=noninteractive apt install -y libcap2-bin;;
    "rhel") yum -y install libcap;;
    "suse") zypper install -y libcap-progs;;
    "arch") pacman --noconfirm -S libcap;;
    "alpine") apk add --no-interactive libcap-setcap;;
    "freebsd") echo "Your OS doesn't support setcap" >&2;;
    *) echo "You need to install setcap manually" >&2
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

install_runuser() {
  case "$DISTRO" in
    "debian") DEBIAN_FRONTEND=noninteractive apt install -y util-linux;;
    "rhel") yum -y install util-linux;;
    "suse") zypper install -y util-linux;;
    "arch") pacman --noconfirm -S util-linux;;
    "alpine") apk add --no-interactive runuser;;
    "freebsd") echo "Your OS might not support runuser" >&2;;
    *) echo "You need to install runuser manually" >&2
  esac
}

determine_distro() {
    local OS="$(uname -s)"
    if [ "$OS" == "Linux" ]; then
      if [ -f /etc/redhat-release ] ; then
        DISTRO=rhel
      elif [ -f /etc/SuSE-release ] ; then
        DISTRO=suse
      elif [ -f /etc/debian_version ] ; then
        DISTRO=debian
      elif [ -f /etc/arch-release ] ; then
        DISTRO=arch
      elif [ -f /etc/alpine-release ] ; then
        DISTRO=alpine
      else
        DISTRO=other
      fi
    elif [ "$OS" == "FreeBSD" ]; then
      DISTRO=freebsd
    else
      DISTRO=other
    fi
}

check_prerequisities() {
    if [ "$(id -u)" != "0" ]; then
      echo 'You need to have root privileges to install Ferron' >&2
      return 1
    fi
    determine_distro
}

add_ferron_debian_repo() {
    DEBIAN_FRONTEND=noninteractive apt update
    DEBIAN_FRONTEND=noninteractive apt install -y curl gnupg2 ca-certificates lsb-release debian-archive-keyring
    curl https://deb.ferron.sh/signing.pgp | gpg --dearmor | tee /usr/share/keyrings/ferron-keyring.gpg >/dev/null
    CODENAME="sid" # Use "sid" as a fallback for Debian derivatives
    if [ "$(lsb_release -is)" = "Debian" ] || [ "$(lsb_release -is)" = "Ubuntu" ]; then
        CODENAME=$(lsb_release -cs)
    fi
    echo "deb [signed-by=/usr/share/keyrings/ferron-keyring.gpg] https://deb.ferron.sh $CODENAME main" | tee /etc/apt/sources.list.d/ferron.list
    DEBIAN_FRONTEND=noninteractive apt update
}

add_ferron_rpm_repo() {
    yum -y makecache
    yum -y install yum-utils
    yum-config-manager --add-repo https://rpm.ferron.sh/ferron.repo
    yum -y makecache
}

install_debian_package() {
    DEBIAN_FRONTEND=noninteractive apt install -y ferron || return 1
}

install_rpm_package() {
    yum -y install ferron || return 1
    systemctl enable ferron || true
    systemctl start ferron || true
}

install_required_packages() {
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
    if (! type systemctl > /dev/null 2>/dev/null) && (! type setcap > /dev/null 2>/dev/null); then
        if [ "$UPDATED_LISTS" != "1" ]; then
          update_package_lists
          UPDATED_LISTS=1
        fi
        install_setcap
        if (! type systemctl > /dev/null 2>/dev/null) && (! type setcap > /dev/null 2>/dev/null); then
            echo "setcap is not installed, but it's required for non-systemd systems. You might install setcap using your package manager." >&2
            return 1
        fi
    fi
    if (! type systemctl > /dev/null 2>/dev/null) && (! type runuser > /dev/null 2>/dev/null); then
        if [ "$UPDATED_LISTS" != "1" ]; then
          update_package_lists
          UPDATED_LISTS=1
        fi
        install_runuser
        if (! type systemctl > /dev/null 2>/dev/null) && (! type runuser > /dev/null 2>/dev/null); then
            echo "runuser is not installed, but it's required for non-systemd systems. You might install runuser using your package manager." >&2
            return 1
        fi
    fi
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

invalid_input() {
  echo 'Invalid input.' >&2
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
    mkdir -p /var/log/ferron || return 1
    mkdir -p /var/lib/ferron || return 1
    mkdir -p /var/www || return 1
    if ! [ -d /var/www/ferron ]; then
      mv $FERRON_EXTRACTION_DIRECTORY/wwwroot /var/www/ferron || return 1
    fi
    mv $FERRON_EXTRACTION_DIRECTORY/ferron{,-*} /usr/sbin || return 1
    chown root:root /usr/sbin/ferron{,-*} || return 1
    chmod a+rx /usr/sbin/ferron{,-*} || return 1
    rm -rf $FERRON_EXTRACTION_DIRECTORY || return 1
    return 0
}

create_ferron_configuration() {
    (cat > /etc/ferron.kdl << 'FERRON_CONFIGURATION_END_OF_FILE'
%{FERRON_CONFIG}
FERRON_CONFIGURATION_END_OF_FILE
    ) || return 1
    chmod a+r /etc/ferron.kdl || return 1
}

create_ferron_user() {
    if type useradd &>/dev/null; then
        useradd -d /var/lib/ferron -m -s /usr/sbin/nologin -r ferron || return 1
    else
        # useradd is not available for instance on Alpine Linux
        addgroup --system ferron || true
        adduser --home /var/lib/ferron --shell /usr/sbin/nologin --system --disabled-password --ingroup ferron ferron || return 1
    fi
    chown -hR ferron:ferron /var/log/ferron || return 1
    chown -hR ferron:ferron /var/lib/ferron || return 1
    chown -hR ferron:ferron /var/www/ferron || return 1
    find /var/log/ferron -type d -exec chmod 755 {} \; || return 1
    find /var/log/ferron -type f -exec chmod 644 {} \; || return 1
    find /var/www/ferron -type d -exec chmod 755 {} \; || return 1
    find /var/www/ferron -type f -exec chmod 644 {} \; || return 1
}

install_updater() {
  (cat > /usr/bin/ferron-updater << 'FERRON_UPDATER_END_OF_FILE'
%{FERRON_UPDATER}
FERRON_UPDATER_END_OF_FILE
    ) || return 1
    chmod a+rx /usr/bin/ferron-updater || return 1
}

fix_selinux_context() {
    restorecon -r /usr/sbin/ferron{,-*} /usr/bin/ferron-updater /etc/ferron.kdl /var/www/ferron /var/log/ferron /var/lib/ferron || return 1
}

install_service() {
    if ! type systemctl > /dev/null 2>&1 || [ -d /etc/init.d ]; then
      (cat > /etc/init.d/ferron << 'EOF'
#!/bin/bash
### BEGIN INIT INFO
# Provides:          ferron
# Required-Start:    $local_fs $remote_fs $network $syslog $named
# Required-Stop:     $local_fs $remote_fs $network $syslog $named
# Default-Start:     2 3 4 5
# Default-Stop:      0 1 6
# X-Interactive:     true
# Short-Description: Ferron web server
# Description:       Start the web server
#  This script will start the Ferron web server.
### END INIT INFO

server="/usr/sbin/ferron"
serverargs="-c /etc/ferron.kdl"
servicename="Ferron web server"

user="ferron"

script="$(basename $0)"
lockfile="/var/lock/$script"

. /etc/rc.d/init.d/functions 2>/dev/null || . /etc/rc.status 2>/dev/null || . /lib/lsb/init-functions 2>/dev/null

ulimit -n 12000 2>/dev/null
RETVAL=0

privilege_check()
{
  if [ "$(id -u)" != "0" ]; then
    echo 'You need to have root privileges to manage Ferron service'
    exit 1
  fi
}

do_start()
{
    if [ ! -f "$lockfile" ] ; then
        echo -n $"Starting $servicename: "
        setcap 'cap_net_bind_service=+ep' $server
        (runuser -u $user -- $server $serverargs > /dev/null &) && echo_success || echo_failure
        RETVAL=$?
        echo
        [ $RETVAL -eq 0 ] && touch "$lockfile"
    else
        echo "$servicename is locked."
        RETVAL=1
    fi
}

echo_failure() {
    echo -n "fail"
}

echo_success() {
    echo -n "success"
}

echo_warning() {
    echo -n "warning"
}

do_stop()
{
    echo -n $"Stopping $servicename: "
    if type ps > /dev/null 2>&1; then
      pid=`ps -aef | grep "$server $serverargs" | grep -v " grep " | awk '{print $2}' | xargs`
    else
      pid=`pidof $server | xargs`
    fi
    kill -9 $pid > /dev/null 2>&1 && echo_success || echo_failure
    RETVAL=$?
    echo
    [ $RETVAL -eq 0 ] && rm -f "$lockfile"

    if [ "$pid" = "" -a -f "$lockfile" ]; then
        rm -f "$lockfile"
        echo "Removed lockfile ( $lockfile )"
    fi
}

do_reload()
{
    echo -n $"Reloading $servicename: "
    if type ps > /dev/null 2>&1; then
      pid=`ps -aef | grep "$server $serverargs" | grep -v " grep " | awk '{print $2}' | xargs`
    else
      pid=`pidof $server | xargs`
    fi
    kill -1 $pid > /dev/null 2>&1 && echo_success || echo_failure
    echo
}

do_status()
{
   if type ps > /dev/null 2>&1; then
     pid=`ps -aef | grep "$server $serverargs" | grep -v " grep " | awk '{print $2}' | head -n 1`
   else
     pid=`pidof -s $server`
   fi
   if [ "$pid" != "" ]; then
     echo "$servicename (pid $pid) is running..."
   else
     echo "$servicename is stopped"
   fi
}

case "$1" in
    start)
        privilege_check
        do_start
        ;;
    stop)
        privilege_check
        do_stop
        ;;
    status)
        do_status
        ;;
    restart)
        privilege_check
        do_stop
        do_start
        RETVAL=$?
        ;;
    reload)
        privilege_check
        do_reload
        ;;
    *)
        echo "Usage: $0 {start|stop|status|restart|reload}"
        RETVAL=1
esac

exit $RETVAL
EOF
    ) || return 1
    chmod a+rx /etc/init.d/ferron || return 1
  fi
if ! type systemctl > /dev/null 2>&1; then
  if type update-rc.d > /dev/null 2>&1; then
    update-rc.d ferron defaults || return 1
  else
    rc-update add ferron default || return 1
  fi
  /etc/init.d/ferron start || return 1
else
  (cat > /etc/systemd/system/ferron.service << 'EOF'
[Unit]
Description=Ferron web server
After=network.target

[Service]
Type=simple
User=ferron
ExecStart=/usr/sbin/ferron -c /etc/ferron.kdl
ExecReload=kill -HUP $MAINPID
Restart=on-failure
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
EOF
  ) || return 1
  systemctl enable ferron || return 1
  systemctl start ferron || return 1
fi
}

# Welcome message
print_welcome

# Execute tasks
do_task 'Checking prerequisites' check_prerequisities
USE_DISTRO_PACKAGE="0"
case "$DISTRO" in
  debian)
    read_input 'It seems like you are using Debian, Ubuntu, or a derivative. Install a Ferron package for Debian? (y/n)' USE_DEBIAN_PACKAGE
    case "$USE_DEBIAN_PACKAGE" in
      y|Y|yes|YES)
        do_task 'Adding Ferron repository' add_ferron_debian_repo
        do_task 'Installing Ferron from a Debian package' install_debian_package
        USE_DISTRO_PACKAGE="1"
        ;;
      n|N|no|NO)
        ;;
      *)
        do_task 'Refusing to install on invalid input' invalid_input
        ;;
    esac
    ;;
  rhel)
    read_input 'It seems like you are using RHEL, Fedora, or a derivative. Install a Ferron RPM package? (y/n)' USE_RPM_PACKAGE
    case "$USE_RPM_PACKAGE" in
      y|Y|yes|YES)
        do_task 'Adding Ferron repository' add_ferron_rpm_repo
        do_task 'Installing Ferron from an RPM package' install_rpm_package
        USE_DISTRO_PACKAGE="1"
        ;;
      n|N|no|NO)
        ;;
      *)
        do_task 'Refusing to install on invalid input' invalid_input
        ;;
    esac
    ;;
esac
if [ "$USE_DISTRO_PACKAGE" != "1" ]; then
    do_task 'Installing required packages' install_required_packages
    read_input 'Installation channel (`stable` for stable, `manual` for manual installation from a ZIP archive)' INSTALLATION_CHANNEL
    if [ "$INSTALLATION_CHANNEL" = "stable" ]; then
        do_task 'Downloading Ferron ZIP archive' download_ferron_zip
    elif [ "$INSTALLATION_CHANNEL" = "manual" ]; then
        read_input 'Path to the Ferron ZIP archive' FERRON_ZIP_ARCHIVE
    else
        do_task 'Refusing to install on invalid installation type' invalid_installation_type
    fi
    do_task 'Copying Ferron files' copy_ferron_files
    do_task 'Creating Ferron configuration' create_ferron_configuration
    do_task 'Installing Ferron updater' install_updater
    do_task 'Creating Ferron user' create_ferron_user
    if type restorecon >/dev/null 2>&1; then
        do_task 'Fixing SELinux context' fix_selinux_context
    fi
    do_task 'Installing Ferron service' install_service
fi

# Ferron is installed successfully
print_installation_success
