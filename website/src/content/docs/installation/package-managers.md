---
title: Installation via package managers
---

Ferron has several community-maintained packages that can be installed via various package managers. Below are the instruction on how to install Ferron via a package manager.

## Homebrew (macOS and GNU/Linux)

To install Ferron via Homebrew, you can run the command below:

```bash
brew install ferron
```

This command installs `ferron` command, which runs a web server.

You can view this Ferron package on [Homebrew Formulae](https://formulae.brew.sh/formula/ferron).

## Nix unstable (GNU/Linux)

To install Ferron via Nix (unstable channel), you can run the command below:

```bash
nix-shell -p ferron
```

This command installs `ferron` command, which runs a web server, and the `ferron-passwd` command, which is a password generation utility for Ferron web server.

You can view this Ferron package on [Nixpkgs](https://search.nixos.org/packages?channel=unstable&show=ferron&from=0&size=50&sort=relevance&type=packages&query=ferron).