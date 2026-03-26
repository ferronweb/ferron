Name:           ferron-lts
Version:        $VERSION
Release:        1%{?dist}
Summary:        A fast, modern, and easily configurable web server with automatic TLS (LTS version).

License:        MIT
URL:            https://ferron.sh

Requires:       glibc >= 2.31
Requires:       ca-certificates
Requires(post): systemd
Requires(pre):  shadow-utils
Requires(postun): systemd
Conflicts:      ferron

Provides:       webserver

%description
A fast, modern, and easily configurable web server with automatic TLS.

%global debug_package %{nil}

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
if [ $1 -eq 1 ]; then
    # Initial installation
    mkdir -p /var/www/ferron
    if [ ! -e /var/www/ferron/index.html ]; then
        cp -r /usr/share/ferron/wwwroot/* /var/www/ferron/
    fi

    chown -R ferron:ferron /var/log/ferron /var/lib/ferron /var/www/ferron
    find /var/www/ferron -type f -exec chmod 644 {} \;
    find /var/www/ferron -type d -exec chmod 755 {} \;
    find /var/log/ferron -type f -exec chmod 644 {} \;
    find /var/log/ferron -type d -exec chmod 755 {} \;

    if type selinuxenabled >/dev/null 2>&1 && selinuxenabled; then
        if type setsebool >/dev/null 2>&1; then
            # ACME and reverse proxy (taken from Caddy's RPM spec)
            setsebool -P httpd_can_network_connect on
        fi

        if type semanage >/dev/null 2>&1; then
            semanage fcontext -a -t httpd_exec_t "/usr/sbin/ferron" 2>/dev/null || semanage fcontext -m -t httpd_exec_t "/usr/sbin/ferron" 2>/dev/null || semanage fcontext -a -t httpd_exec_t "/usr/bin/ferron" 2>/dev/null || semanage fcontext -m -t httpd_exec_t "/usr/bin/ferron" || :
            semanage fcontext -a -t httpd_config_t "/etc/ferron.kdl" 2>/dev/null || semanage fcontext -m -t httpd_config_t "/etc/ferron.kdl" || :
            semanage fcontext -a -t httpd_sys_content_t "/var/www/ferron(/.*)?" 2>/dev/null || semanage fcontext -m -t httpd_sys_content_t "/var/www/ferron(/.*)?" || :
            semanage fcontext -a -t httpd_log_t "/var/log/ferron(/.*)?" 2>/dev/null || semanage fcontext -m -t httpd_log_t "/var/log/ferron(/.*)?" || :
            semanage fcontext -a -t httpd_var_lib_t "/var/lib/ferron(/.*)?" 2>/dev/null || semanage fcontext -m -t httpd_var_lib_t "/var/lib/ferron(/.*)?" || :
        fi

        if type restorecon >/dev/null 2>&1; then
            restorecon -r /usr/sbin/ferron{,-*} /etc/ferron.kdl /var/www/ferron /var/log/ferron /var/lib/ferron || :
        fi

        if type semanage >/dev/null 2>&1; then
            # QUIC (taken from Caddy's RPM spec)
            semanage port -a -t http_port_t -p udp 80 2>/dev/null || semanage port -m -t http_port_t -p udp 80 || :
            semanage port -a -t http_port_t -p udp 443 2>/dev/null || semanage port -m -t http_port_t -p udp 443 || :
        fi
    fi
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
    if type selinuxenabled >/dev/null 2>&1 && selinuxenabled; then
        if type setsebool >/dev/null 2>&1; then
            # ACME and reverse proxy (taken from Caddy's RPM spec)
            setsebool -P httpd_can_network_connect off
        fi

        if type semanage >/dev/null 2>&1; then
            semanage fcontext -d "/usr/sbin/ferron" 2>/dev/null || semanage fcontext -d "/usr/bin/ferron" || :
            semanage fcontext -d "/etc/ferron.kdl" || :
            semanage fcontext -d "/var/www/ferron(/.*)?" || :
            semanage fcontext -d "/var/log/ferron(/.*)?" || :
            semanage fcontext -d "/var/lib/ferron(/.*)?" || :
        fi

        if type restorecon >/dev/null 2>&1; then
            restorecon -r /usr/sbin/ferron{,-*} /etc/ferron.kdl /var/www/ferron /var/log/ferron /var/lib/ferron || :
        fi

        if type semanage >/dev/null 2>&1; then
            # QUIC (taken from Caddy's RPM spec)
            semanage port -d -t http_port_t -p udp 80 || :
            semanage port -d -t http_port_t -p udp 443 || :
        fi
    fi
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
