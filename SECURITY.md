# Ferron Security Policy

## Overview
Ferron is a fast, memory-safe web server written in Rust, designed for performance and security. This document outlines the security policies and procedures to ensure Ferron remains a secure and reliable software project.

## Supported versions
Ferron actively supports the latest stable release and provides security updates for the most recent minor versions. Users are encouraged to upgrade promptly to receive security patches.

## Reporting security issues
Security is a top priority for Ferron. If you discover a vulnerability, please report it responsibly by sending an email message to [security@ferron.sh](mailto:security@ferron.sh).

We strongly discourage public disclosure of vulnerabilities before a fix is released.

## Security best practices
To maintain security, we follow these principles:

- **Memory safety** - Ferron leverages Rust’s ownership model and borrow checker to eliminate memory-related vulnerabilities.
- **Minimal attack surface** - features are enabled only as needed, reducing exposure to potential threats.
- **Regular audits** - code is reviewed regularly, and dependencies are monitored for security vulnerabilities.
- **Safe defaults** - Ferron has some insecure configuration disabled by default, like exposing the server version or directory listings.

## Secure development process
Ferron follows industry best practices to maintain a secure development lifecycle:

1. **Code review** - all changes undergo peer review with security checks.
2. **Dependency management** - regularly check and update dependencies to patch known vulnerabilities.
3. **Responsible disclosure** - work with the security community to resolve issues before public disclosure.

## Handling security incidents
In the event of a security breach or vulnerability:

1. **Triage** - assess and prioritize the issue based on severity.
2. **Mitigation** - develop and test a fix.
3. **Advisory** - issue a security advisory with mitigation steps and fixed versions.
4. **Update users** - notify users via release notes and security mailing lists.

## Contact information
For any security concerns, contact us at [security@ferron.sh](mailto:security@ferron.sh). Stay updated on security patches via [our website](https://www.ferronweb.org).

By following this policy, we ensure Ferron remains a secure and trustworthy web server for all users.
