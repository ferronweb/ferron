Name:           ferron
Version:        $VERSION
Release:        1%{?dist}
Summary:        A fast, modern, and easily configurable web server with automatic TLS.

License:        MIT
URL:            https://ferron.sh

Requires:       glibc >= 2.31
Requires:       ca-certificates
Requires(post): systemd
Requires(pre):  shadow-utils
Requires(postun): systemd

BuildArch:      $ARCHITECTURE

Provides:       webserver

%description
A fast, modern, and easily configurable web server with automatic TLS.

%prep
%setup -q

%build
# No build commands necessary, since binaries are precompiled

%install
mkdir -p %{buildroot}/usr/sbin
mkdir -p %{buildroot}/usr/lib/systemd/system
mkdir -p %{buildroot}/usr/share/ferron/wwwroot
mkdir -p %{buildroot}/var/log/ferron
mkdir -p %{buildroot}/var/lib/ferron
mkdir -p %{buildroot}/etc

install -m 0755 data/usr/sbin/ferron %{buildroot}/usr/sbin/ferron
install -m 0755 data/usr/sbin/ferron-passwd %{buildroot}/usr/sbin/ferron-passwd
install -m 0755 data/usr/sbin/ferron-precompress %{buildroot}/usr/sbin/ferron-precompress
install -m 0755 data/usr/sbin/ferron-yaml2kdl %{buildroot}/usr/sbin/ferron-yaml2kdl

install -m 0644 data/etc/ferron.kdl %{buildroot}/etc/ferron.kdl

install -m 0644 data/usr/lib/systemd/system/ferron.service %{buildroot}/usr/lib/systemd/system/ferron.service

cp -r data/usr/share/ferron/wwwroot %{buildroot}/usr/share/ferron/

%pre
if ! id ferron &>/dev/null; then
    useradd -r -d /var/lib/ferron -s /sbin/nologin ferron
fi

%post
mkdir -p /var/www/ferron
if [ ! -e /var/www/ferron/index.html ]; then
    cp -r /usr/share/ferron/wwwroot/* /var/www/ferron/
fi

chown -R ferron:ferron /var/log/ferron /var/lib/ferron /var/www/ferron
chmod -R 755 /var/log/ferron /var/www/ferron
chmod -R 644 /var/log/ferron/* /var/www/ferron/* 2>/dev/null || true

# TODO: proper SELinux support
if type restorecon >/dev/null 2>&1; then
    restorecon -r /usr/sbin/ferron{,-*} /etc/ferron.kdl /var/www/ferron /var/log/ferron /var/lib/ferron
fi

# systemd macros aren't used, so that the RPM package can be built on Debian-based systems
if [ $1 -eq 1 ] && [ -x "/usr/lib/systemd/systemd-update-helper" ]; then
    # Initial installation
    /usr/lib/systemd/systemd-update-helper install-system-units ferron.service || :
fi

%preun
if [ $1 -eq 0 ] && [ -x "/usr/lib/systemd/systemd-update-helper" ]; then
    # Package removal, not upgrade
    /usr/lib/systemd/systemd-update-helper remove-system-units ferron.service || :
fi

%postun
if [ $1 -eq 0 ]; then
    rm -rf /var/lib/ferron /var/log/ferron /etc/ferron
fi

%files
%defattr(-,root,root,-)
/usr/sbin/ferron
/usr/sbin/ferron-passwd
/usr/sbin/ferron-precompress
/usr/sbin/ferron-yaml2kdl
/usr/lib/systemd/system/ferron.service
/usr/share/ferron/wwwroot
%dir /var/log/ferron
%dir /var/lib/ferron
%ghost /var/www/ferron
%config(noreplace) /etc/ferron.kdl
