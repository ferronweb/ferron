---
title: "How to host a website on a VPS?"
description: "Learn how to host a website on a VPS server, step by step, and the differences from different hosting types."
date: 2025-07-16 22:22:00
cover: /img/covers/how-to-host-a-website-on-a-vps.png
---

When starting a website, you will need to get web hosting. Web hosting is where the website is stored and made accessible on the internet for visitors. There are many web hosting types, such as shared hosting, cloud hosting and VPS hosting.

A VPS (Virtual Private Server) is a server virtualized on a physical server, sold by a hosting company as a service. Hosting a website on a VPS can give you more control, flexibility and performance, compared to shared hosting.

In this post, you will learn how to host a website on a VPS server, step by step, and the differences from different hosting types.

## How a VPS differs from shared hosting?

A VPS and shared hosting are both different types of web hosting, with some differences.

Shared hosting has server resources (such as CPU or memory) shared across the websites, this allows cost reduction. However, if one website consumes a lot of server resources, other websites can slow down. As a result, website visitors might more often bounce away, and search engine ranking might get decreased.

A VPS however, has dedicated resources; while multiple VPS servers can run on the same physical server, potentially sharing resources, each VPS has its own allocated resources. This ensures better performance and stability for the services.

Also, shared hosting allows only limited control over server settings and configurations. For example, many shared hosting plans use the LAMP (GNU/Linux, Apache httpd, MariaDB, PHP) stack, but many websites and web applications use different stacks. In comparison, a VPS allows multiple technology stacks and variations of software to be installed. A VPS can also offer root access, allowing for installing custom software.

## How a VPS differs from PaaS (like Vercel)?

A VPS and shared hosting are both different ways to host websites, with many differences.

With PaaS (Platform as a Service; for example Vercel), you don't need to worry about the underlying infrastructure; the platform handles server management, scaling and maintenance. However, this can be limiting for someone looking for bigger control.

With a VPS however, there is bigger control over the server, allowing more technology stacks than what would be allowed on a PaaS.

A PaaS often follows a pay-as-you-go model, where customers are charged based on consumed resources. However, when there can be a lot of usage, [there can be a lot of cost](https://serverlesshorrors.com/all/vercel-23k/). When you have a VPS however, you pay a fixed periodic fee, regardless on the usage, which can be cheap on the longer run.

## Prerequisities

When hosting a website on a VPS, or even when hosting a website, a domain name is often used. When it's possible access a website directly via an IP address, the IP address might be diffcult to remember (what would you remember better: `www.ferronweb.org`, or `194.110.4.248`?). A domain name can be easier to remember, and can be also branded.

Also, some basic knowledge of SSH and GNU/Linux system administration can be useful to set up a website on a VPS. You might experience installation of packages, or configuring a web server, so this knowledge can help you.

VPS servers (GNU/Linux ones) often provide SSH access, so you can administrate the server, for example install services, configure them, update the operating system and software.

This post assumes you're using GNU/Linux as an operating system for your VPS server, and you're about to host a static website. The steps for hosting a website on a VPS can vary from website to website.

## Choosing the right VPS provider

Choosing the right VPS provider and the right VPS plan is important, because the right VPS server would be both affordable, and have enough resources to run a website.

One example of a VPS provider is [awHost](https://awhost.pl/) (we're using a VPS server from them!), which is a Polish VPS hosting provider. awHost offers variety of affordable plans, starting from around $5.08 per month when billed monthly for a 2 GB RAM, 1 vCPU, 20 GB disk space VPS (of course excluding the "sandbox" VPS plan).

Another example of a VPS provider can be [Altivox Networks](https://altivox.net/). Altivox also offers affordable VPS hosting plans, starting from $5 per month for a 4 GB RAM, 2 vCPU, 200 GB disk space VPS.

## Step-by-step guide

### 1. Buy a domain name

First, buy a domain name for a website address which can be easy to remember. You can purchase it on [Porkbun](https://porkbun.com/) (we have registered our domain name here).

### 2. Choose a VPS plan and set up a VPS

After buying a domain name, choose a VPS plan according to your website's needs. You can choose a VPS from provider we have listed in a "Choosing the right VPS provider" section above.

After getting a VPS plan, you can use the VPS provider's setup tool. At this time, your VPS server will get provisioned.

### 3. Point the domain name to the VPS

First, check the IP address of a VPS server, this can be found on a VPS provider's panel or in the login data. Then, log into the domain registrar's panel (or DNS hosting panel), and add an A record (or additionally an AAAA record if your VPS has an IPv6 address) pointing a domain name to your VPS server's IP address.

You might have to delete some existing records set by default that point to other servers (A, AAAA, ALIAS, CNAME records).

### 4. Connect to your VPS via SSH

Connect to your VPS server via SSH. The credentials for log into a VPS server can be found in an email message sent from the VPS provider or in a VPS hosting provider's panel.

You can use this command to log into your VPS server:

```bash
# Replace `2022` with your VPS's SSH port, `192.168.1.1` with your VPS's IP address, and `root` with the VPS login username. If using port 22, remove `-p 2022`.
ssh -p 2022 root@192.168.1.1
```

When you log into a VPS, you might be prompted to enter a password, unless you uploaded an SSH key for passwordless login in a VPS provider's setup tool. Enter the password from the VPS login data.

### 5. Secure your VPS server

Securing a VPS server is important, because bad actors trying to attack your VPS server can appear. You can follow [these tips to secure your VPS server](/blog/8-tips-to-secure-your-gnu-linux-vps/).

### 6. Install a web server

A web server is software responsible for serving your website. There are many web server software choices, and choosing the right web server based on the performance and ease of use is important.

For this post, we will go with Ferron, which is a fast, easily-configurable web server. It can even obtain TLS certificates automatically, so you don't need to think much about configuring HTTPS for securing your website.

You can use this command to install Ferron web server (Ferron 2.x):

```bash
sudo bash -c "$(curl -fsSL https://downloads.ferronweb.org/install-v2.sh)"
```

If you want to install Ferron 1.x, you can use this command:

```bash
sudo bash -c "$(curl -fsSL https://downloads.ferronweb.org/install.sh)"
```

Follow the prompts appearing on the terminal when installing the web server.

After installing the web server, you can check if it's working by opening a web browser, and typing in the domain name pointing to your VPS in the address bar. If you see a webpage saying "Ferron is installed successfully" or a similar page, you have successfully installed a web server.

### 7. Configure the web server

When hosting a website on a VPS server, web server configurations can vary depending on the website.

In this post, the website will be a simple static website (the website is served directly from the website files).

You can use this Ferron 2.x configuration (in `/etc/ferron.kdl`) for static files:

```kdl
* {
  log "/var/log/ferron/access.log"
  error_log "/var/log/ferron/error.log"
}

// Replace "example.com" with your domain name pointing to your VPS
example.com {
  // Replace "/var/www/ferron" with path to the folder, where the website files will be uploaded
  root "/var/www/ferron"
}
```

If you're using Ferron 1.x, you can use this configuration (in `/etc/ferron.yaml`):

```yaml
global:
  secure: true
  enableAutomaticTLS: true
  automaticTLSContactCacheDirectory: "/var/cache/ferron-acme" # Replace "/var/cache/ferron-acme" with the path to the ACME cache directory. Change the owner of the ACME cache directory to the `ferron` user.
  logFilePath: /var/log/ferron/access.log
  errorLogFilePath: /var/log/ferron/error.log

hosts:
  - domain: "example.com" # Replace "example.com" with your domain name pointing to your VPS
    wwwroot: "/var/www/ferron" # Replace "/var/www/ferron" with path to the folder, where the website files will be uploaded
```

This configuration describes that a web server will serve website files for a website with a domain name pointing to your VPS, and log the requests and errors into log files for easier troubleshooting. The web server will be also configured to automatically obtain TLS certificates for the website from Let's Encrypt, so you don't need to think much about setting up HTTPS.

You can edit the server configuration using `nano` command, followed by the server configuration file path.

You can also read more about configuring Ferron in either [Ferron 2.x documentation](https://v2.ferronweb.org/docs) or in the [Ferron 1.x documentation](https://www.ferronweb.org/docs).

After you configure the web server, restart the web server using either `sudo systemctl reload ferron` or `sudo /etc/init.d/ferron reload` command.

### 8. Upload the website files

After configuring the web server, upload the website files using your preferred SFTP client. You can use [FileZilla](https://filezilla-project.org/) for uploading the website files to your VPS server. Upload the website files to the directory, with the path specified in the web server configuration.

### 9. Test your website

After uploading the website files, you can test your website to ensure everything is working correctly. You can test your website by typing in the domain name pointing to your VPS in the address bar in your web browser. If you see the intended page (and not a server error page or a browser error page), you website loads correctly, and you can even test the website further.

## Common issues and troubleshooting

When hosting a website on a VPS server, there might be some issues setting up the website.

### SSH login problems

If you can't log into your VPS server due to incorrect credentials, make sure to double-check the username and password. If you're using SSH keys, make sure to use the correct private key.

You might also add your SSH key to the SSH server using this command:

```bash
# Replace `2022` with your VPS's SSH port, `192.168.1.1` with your VPS's IP address, and `root` with the VPS login username. If using port 22, remove `-p 2022`.
ssh-copy-id -p 2022 root@192.168.1.1
```

Sometimes, you can't connect to your VPS server at all, in this case, double-check the VPS server's IP address. You might also have problems with configuring the firewall, in this case you can log into your VPS via a web console, if available, or even reinstall the operating system and provision the VPS servers again.

In some other cases though, your network might block connecting to the SSH service, in this case, you can try logging into your VPS server from a different network.

### Website connectivity issues

If you can't connect to the website, there might be several reasons for website connectivity issues.

One possible issue is that the website address couldn't be resolved. In this case, check the website address, or if you have set up DNS records pointing correctly.

Another possible reason is that the domain name points to the wrong IP address; this might lead to connection timeouts. In this case check your VPS server's correct IP address, and point your domain to the right IP address.

If you see an issue related to TLS certificates (such as "Your connection is not private" errors), check your web server configuration, and if applicable, renew the TLS certificates.

If you see a web server error page (such as 500 Internal Server Error page), you can check the web server logs to see the errors that occurred when trying to access the website. Also, you can check the server configuration for possible misconfigurations.

## Can I host multiple websites on one VPS?

Yes, you can host multiple websites on one VPS server by specifying a host in the web server configuration and pointing multiple domain names to the VPS server.

You need to also make sure your VPS server has enough resources to handle traffic for multiple websites.

## Conclusion

Hosting a website on a VPS gives you the perfect balance of flexibility, control, and performance, especially compared to shared hosting or managed platforms like Vercel. While it does require a bit more technical knowledge, the rewards are worth it: you get full control over your environment, the ability to run custom stacks, and often more predictable pricing.

By following the steps outlined in this post, from choosing a VPS provider and domain name, to configuring your web server and uploading your website, you’ll have a solid foundation for managing your own hosting setup. Whether you're running a personal project, a portfolio, or even a business site, hosting on a VPS can be a powerful and affordable solution.

If you're ready to take your website hosting to the next level, give VPS hosting a try, you might be surprised how far a little control can take you.
