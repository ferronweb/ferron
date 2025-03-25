---
title: 8 tips to secure your GNU/Linux VPS
description: "Secure your GNU/Linux VPS with these essential steps. Protect against DDoS, update software, secure SSH, and more to safeguard your data."
date: 2025-03-25 11:38:00
cover: /img/covers/8-tips-to-secure-your-gnu-linux-vps.png
---

Security is essential for a GNU/Linux VPS (virtual private server) because attackers actively search for vulnerable servers to exploit.

Imagine hosting a website or some other service hosted on a VPS, and an attacker guesses the default password or exploits a known vulnerability, installs malware, and now what? Your customers' data might be in danger, and you risk damaging your reputation.

In this post, you will explore the ways to secure a GNU/Linux VPS to ensure that you and your customers are safe.

## Use a VPS with DDoS protection

DDoS (Distributed Denial-of-Service) attacks involve many network devices flooding a target with many concurrent requests to disrupt the services provided by the target server. These attacks prevent legitimate users from using the services. Those users may then leave using your service, therefore it's important to use a VPS that offers protection against DDoS attacks.

Many hosting services offering VPSes also offer DDoS protection. For example, [awHost](https://awhost.pl) (we are using their VPS to host this website!; the website is in Polish though), offers anti-DDoS protection for their VPSes via selective blackholing (Polish location) or OVH's advanced anti-DDoS protection (French location).

## Update software to the latest versions

Updating software to the latest versions is important, because updates ofter include patches for vulnerability that could be exploited by attackers.

To update software to latest version, run the commands below (on Debian/Ubuntu servers):

```bash
sudo apt update
sudo apt upgrade
```

You can also configure automatic software updates for Debian/Ubuntu servers. To do this first install `unattended-upgrades` package:

```bash
sudo apt install unattended-upgrades
```

After installing this package, reconfigure the package using the command below:

```bash
sudo dpkg-reconfigure unattended-upgrades
```

To enable unattended upgrades, select the "Yes" option at the packet configuration screen.

## Secure the SSH service

SSH (Secure Shell) service allows server administrators to securely access and manage remote servers.

Unfortunately, many attackers are exploiting misconfigurations and weak credentials for SSH services to gain unauthorized access to systems. This may lead your customers' data to be in danger, and disruption of your services.

There are many configuration options for securing the SSH service.

### Change the port for the SSH service

Many automated attacks scan and target the default SSH port (port 22). By changing the SSH port, you can reduce the probability of being targeted by these attacks.

If you're using OpenSSH, change the SSH service configuration (for example at `/etc/ssh/sshd_config`) to include this directive:

```
# Replace "2222" with your chosen port
Port 2222
```

After changing the configuration, reload the configuration of the SSH service:

```bash
sudo /etc/init.d/sshd reload # Non-systemd systems
sudo systemctl reload sshd # Systemd systems
```

### Disallow root user logins

The `root` account has unrestricted access to all commands and file on a system. If an attacker gains access to this account, they can compromise the entire system. By disabling logins to the `root` account, you reduce the risk of compromising the system.

If you don't have an non-root non-system user, create it using the command below:

```bash
# Replace "sysadmin" with your chosen username
useradd -m -s /bin/bash sysadmin
passwd sysadmin
```

If you don't have `sudo` installed on your system, you can install it using the command below (for Debian/Ubuntu servers):

```bash
# Run it as root
apt install sudo
```

After installing `sudo`, add the user you have created to the `sudo` group, to ensure that you can elevate your privileges to `root`:

```bash
# Replace "sysadmin" with your chosen username
# Run it as root
usermod -aG sudo sysadmin
```

If you're using OpenSSH, change the SSH service configuration (for example at `/etc/ssh/sshd_config`) to include this directive:

```
PermitRootLogin no
```

After changing the configuration, reload the configuration of the SSH service:

```bash
sudo /etc/init.d/sshd reload # Non-systemd systems
sudo systemctl reload sshd # Systemd systems
```

### Don't permit empty passwords

An empty password allows anyone to attempt to log in without any authentication. This creates a significant security risk, as unauthorized users can easily gain access to the system.

If you're using OpenSSH, change the SSH service configuration (for example at `/etc/ssh/sshd_config`) to include this directive:

```
PermitEmptyPasswords no
```

After changing the configuration, reload the configuration of the SSH service:

```bash
sudo /etc/init.d/sshd reload # Non-systemd systems
sudo systemctl reload sshd # Systemd systems
```

### Enable public-key authentication and disable password authentication

Password authentication is vulnerable to brute force attacks (where an attacker searches for many username/password combinations, until they find a correct one). Enabling public-key authentication and disabling password authentication forces the attacker to have access to the private key, which they couldn't guess easily.

First, generate the SSH key and copy it into the SSH service using commands below:

```bash
# Replace "22" with your chosen port
# Replace "sysadmin@vps.example" with the username and the VPS address combination.
ssh-keygen
ssh-copy-id -p 22 sysadmin@vps.example
```

You can later test if public-key authentication is working or not using the command below:

```bash
# Replace "22" with your chosen port
ssh -p 22 sysadmin@vps.example
```

If public-key authentication is working, log into the VPS, and if you're using OpenSSH, change the SSH service configuration (for example at `/etc/ssh/sshd_config`) to include this directive:
```
PasswordAuthentication no
```

After changing the configuration, reload the configuration of the SSH service:

```bash
sudo /etc/init.d/sshd reload # Non-systemd systems
sudo systemctl reload sshd # Systemd systems
```

## Install and enable a firewall

Installing and enabling a firewall is important when it comes to securing a VPS, because you can control the flow of traffic. You can set rules to allow or deny traffic to specific IP addresses, protocols, and ports.

To install a firewall (we use UFW), first run the command below (for Debian/Ubuntu servers):

```bash
sudo apt install ufw
```

After installing a firewall, allow traffic to SSH, and other services:

```bash
sudo ufw allow ssh # If using SSH port other than 22, replace "ssh" with something like "2222/tcp" if using port 2222
sudo ufw allow http # If you are hosting a website
sudo ufw allow https # If you are hosting a website
```

After allowing the traffic, enable the firewall using the command below:

```bash
sudo ufw enable
```

## Install and configure FailBan

[Fail2Ban](https://github.com/fail2ban/fail2ban) is a daemon that scans and analyzes log files, and bans hosts that performed attacks, based on the log files, reducing the need for manual intervention.

To install Fail2Ban, run the command below (for Debian/Ubuntu servers):

```bash
sudo apt install fail2ban
```

After installing FailBban, enable the SSH filter. Locate the Fail2Ban jail configuration (for example at `/etc/fail2ban/jail.conf`), copy it (for instance to `/etc/fail2ban/jail.local`), find the SSH filter (find for `[sshd]`), and enable it by appending a line below `[sshd]`:

```
enabled = true
```

After enabling the SSH filter, restart Fail2Ban:

```bash
sudo /etc/init.d/fail2ban restart # Non-systemd systems
sudo systemctl restart fail2ban # Systemd systems
```

You might also need to enable other Fail2Ban filters, depending on the services you host on your VPS.

## Install port scanning detection

Port scanning is often a precursor to more serious attacks. By installing and enabling port scanning detection, you can detect port scans, have insights into potential threats, and take measures to secure your server.

To install port scannning detection, first install psad using the command below (for Debian/Ubuntu servers):

```bash
sudo apt install psad
```

Next, reconfigure UFW user rules. Remove all occurrences of ` -m limit --limit 3/min --limit-burst 10` from user rule files for both IPv4 (for example at `/etc/ufw/user.rules`) and IPv6 (for example at `/etc/ufe/user6.rules`). This will ensure all blocked traffic will be logged, so that psad could detect port scans.

After reconfiguring UFW, restart the firewall:

```bash
sudo /etc/init.d/ufw restart # Non-systemd systems
sudo systemctl restart ufw # Systemd systems
```

You can check who performed port scanning using the command below:

```bash
sudo psad -S
```

It's even possible to configure Fail2ban to analyze psad logs and perform various actions.

## Install rootkit detection

Rootkits are type of malware that allows an attacker to take control on a system, and hides its existence from normal detection methods. Rootkits can also alter system files and processes, leading to instability and unpredicatable behavior. That is why it's important to install rootkit detection on your VPS.

To install rootkit detection, run the command below (for Debian/Ubuntu systems):

```bash
sudo apt install chkrootkit
```

To view the latest rootkit scan log, run the command below:

```bash
cat /var/log/chkrootkit/log.today
```

## Use Lynis to audit the VPS

Auditing the system security in a VPS is important, because it helps identify vulnerabilties in the system that could be exploited by attackers.

[Lynis](https://cisofy.com/lynis/) is a tool that perforam a scan of the system to check for system hardening and compliance testing.

To install Lynis, run the command below (for Debian/Ubuntu systems):

```bash
sudo apt install lynis
```

To perform an audit with Lynis, run the command below:

```bash
sudo lynis audit system
```

Lynis will then display the hardening index, suggest what to harden, to ensure the security of the VPS.

## Conclusion

In conclusion, securing a GNU/Linux VPS is a multi-faceted process that requires vigilance and proactive measures. By choosing a VPS with DDoS protection, keeping software up-to-date, securing SSH access, and utilizing tools like firewalls, Fail2Ban, and Lynis, you can significantly improve the security of your VPS. These steps not only protect your data and services but also build trust with your customers. Regular audits and monitoring are essential to maintain a secure environment and mitigate potential threats. By following these best practices, you can ensure a robust defense against cyber attacks and safeguard your digital assets effectively.