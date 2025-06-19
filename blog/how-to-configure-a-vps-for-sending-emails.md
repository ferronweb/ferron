---
title: How to configure a VPS for sending emails?
description: "Learn how to configure a VPS for sending emails, set up the hostname, the SPF record, and the SMTP server."
date: 2025-06-19 06:57:00
cover: /img/covers/how-to-configure-a-vps-for-sending-emails.png
---

If you have a VPS, you might prefer not to pay for an external service for sending email messages (Amazon SES, Mailgun, SendGrid, Mailchimp, and so on) - it might be cheaper to just send email messages directly from a VPS server.

In this post, you will learn how to set up a VPS for sending emails without any external service.

## Requirements

Setting up a VPS for sending emails requires that port 25 must be unblocked on the VPS, otherwise it wouldn't be possible for VPS to communicate with recipients' email servers.

To check whether port 25 is blocked, you can use this command (for this example we're using mail servers for Google Workspace):

```bash
nc aspmx.l.google.com 25
```

If a banner appears, port 25 is unblocked. If it doesn't appear, either the port 25 is blocked (making the VPS unable to send emails directly), or the mail server might be down (you might then try another mail server).

Sometimes, your provider prohibits sending emails from your VPS, in this case, you cannot really use your VPS for sending emails.

This post assumes that the VPS runs a Debian-based system.

## Email blocklist check

Checking the email blocklists is important, because mail servers use them to determine if the IP address is known for sending spam. You probably don't want to use a VPS which has an IP address known for sending spam.

To check if the IP address is listed in email blocklists, you can use [MxToolbox](https://mxtoolbox.com/blacklists.aspx) by entering the VPS's IP address and clicking the green button.

If your VPS's IP address is listed in one of the email blocklists, it's not recommended to configure this VPS for sending emails, as there's higher probability that recipient's email server marks the message as spam or rejects it outright.

## Hostname setup

Setting the host, DNS, and reverse DNS names are all important to improve the email deliverability and to boost credibility of the sender.

First, set the hostname of the VPS by modfiying the `/etc/hostname` file to contain the desired hostname, and afterwards restarting the VPS:

```bash
sudo nano /etc/hostname
sudo reboot
```

After setting the VPS hostname, go to the DNS panel, and add DNS records that point the VPS hostname to the VPS itself - A for IPv4 addresses, AAAA for IPv6 addresses.

You can obtain the IP address of the VPS by running this command on the VPS:

```bash
curl -4 http://ipconfig.io # IPv4
curl -6 http://ipconfig.io # IPv6
```

After adding A and/or AAAA records that point to the VPS, set up reverse DNS for your VPS IP address to match the hostname. You might find a reverse DNS setting in VPS hosting provider's panel.

## SPF setup

SPF (Sender Policy Framework) defines what hosts can send email messages under a specific domain, protecting the domain from email spoofing.

To configure the SPF record, go to the DNS panel, and add a TXT record under the domain you would like to use for email.

The TXT record can contain (replace "127.0.0.1" with the IPv4 address belonging to the VPS):

```
"v=spf1 ip4:127.0.0.1 ~all"
```

If your VPS has IPv6 address, the TXT record can contain (replace "127.0.0.1" with the IP address belonging to the VPS and "::1" with the IPv6 address belonging to the VPS):

```
"v=spf1 ip4:127.0.0.1 ip6:::1 ~all"
```

If your SPF record also includes other allowed hosts (for example custom domain email hosting), like this:

```
"v=spf1 include:zohomail.eu ~all"
```

You can modify the SPF record to include the VPS as well, like this (replace "127.0.0.1" with the IPv4 address belonging to the VPS):

```
"v=spf1 ip4:127.0.0.1 include:zohomail.eu ~all"
```

## SMTP server setup

### Direct installation

To install the SMTP server, run these commands:

```bash
sudo apt update
sudo apt install exim4-daemon-light
```

After installing the SMTP server, run this command:

```bash
sudo dpkg-reconfigure exim4-config
```

After entering this command, you will be prompted several times. Answer the prompts like this:

- **General type of mail configuration** - "internet site; mail is sent and received directly using SMTP"
- **System mail name** - your VPS's hostname
- **IP-addresses to listen on for incoming SMTP connections** - `127.0.0.1 ; ::1`
- **Other destination for which mail is accepted** - leave it empty
- **Domains to relay mail for** - leave it empty
- **Machines to relay mail for** - leave it empty
- **Keep number of DNS-queried minimal (Dial-on-Demand)?** - no
- **Delivery method for local mail** - mbox format in /var/mail/
- **Split configuration into small files?** - no

The application can then be configured with "127.0.0.1" as the server address, 25 as the port, and SSL/TLS disabled.

### Docker Compose

You can define an SMTP container for other Docker containers to be able to send email messages in the "docker-compose.yml" file:

```yaml
services:
  # Some other services
  # ...

  smtp:
    image: devture/exim-relay
    user: 100:101
    restart: always
    environment:
      HOSTNAME: web-pl.ferronweb.org # Replace "web-pl.ferronweb.org" with your VPS's hostname
```

The application can then be configured with "smtp" as the server address, 8025 as the port, and SSL/TLS disabled.

## DKIM setup

You can also set up DKIM (DomainKeys Identified Mail) in [a blog post about using DKIM in exim](https://mikepultz.com/2010/02/using-dkim-in-exim/), if using Exim as an SMTP server.

For updated instructions on setting up DKIM with Exim, you can check the official Exim documentation or reliable community resources.

## Testing the email sending setup

It's important to test the email sending setup before deploying it to reduce the risk of issues in production.

If you have directly installed the SMTP server, you can run this command to test the email sending setup (replace "test@gmail.com" with your email address and "test@example.com" with a test sender email address with the configured domain name):

```bash
echo "Test" | mail -s Test -r test@example.com test@gmail.com
```

If you see the "Test" message in your inbox, that means the test had passed.

## Conclusion

Configuring a VPS for sending emails can be a cost-effective solution for those looking to manage their email communications without relying on external services. By following the steps outlined in this guide, you can ensure that your VPS is properly set up to send emails reliably and securely.

From checking port accessibility and verifying your IP against email blocklists to configuring essential components like SPF, DKIM, and your SMTP server, each step plays an important role in enhancing your email deliverability and protecting your domain's reputation.

Remember to conduct thorough testing of your email setup to identify and resolve any potential problems before going live. With the right configuration, your VPS can serve as a powerful tool for managing your email needs, providing you with greater control and flexibility in your communications.
