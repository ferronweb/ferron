# Ferron security policy

Ferron is a fast, modern web server built for production debugging. This document describes the security policies and procedures to make sure Ferron stays secure and reliable.

## Supported versions

Ferron supports the latest stable release and provides security updates for the most recent minor versions. Users are encouraged to upgrade promptly to receive security patches.

Ferron supports following versions:

| Major version | Status                                                              |
| ------------- | ------------------------------------------------------------------- |
| Ferron 3      | Supported (release candidate)                                       |
| Ferron 2 LTS  | Supported (long term support)                                       |
| Ferron 2      | Supported (stable)                                                  |
| Ferron 1.x    | [End of Life (July 1, 2026)](https://ferron.sh/blog/ferron-1-x-eol) |

## Reporting security issues

Security is very important for Ferron. If you discover a vulnerability, please report it responsibly by sending an email message to [security@ferron.sh](mailto:security@ferron.sh), or [reporting it privately via GitHub](https://github.com/ferronweb/ferron/security/advisories/new).

We strongly discourage public disclosure of vulnerabilities before a fix is released.

## Security best practices

To maintain security, we follow these principles:

- **Memory safety**: Ferron is written in Rust, so whole classes of vulnerabilities are eliminated by the language's memory safety features.
- **Regular audits**: code is reviewed regularly, and dependencies are monitored (using `cargo audit`) for security vulnerabilities.
- **Safe defaults**: Ferron has sensible defaults, so insecure configuration is disabled by default, like exposing the server version or directory listings.
- **Minimal attack surface**: features are enabled only as needed (via web server configuration), reducing exposure to potential threats.

## Secure development process

Ferron follows industry best practices to maintain a secure development lifecycle:

1. **Code review**: all changes (including AI-generated ones!) are reviewed with security and correctness checks.
2. **Dependency management**: regularly check and update dependencies to patch known vulnerabilities.
3. **Responsible disclosure**: work with the security community to resolve issues before public disclosure.

## Contact information

For any security concerns, contact us at [security@ferron.sh](mailto:security@ferron.sh). Stay updated on security patches via [our website](https://ferron.sh).

By following this policy, we make sure Ferron remains a secure and trustworthy web server for everyone.
