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

## `yay` (from AUR; Arch Linux)

To install Ferron from AUR (Arch User Repository), you can run the command below:

```bash
yay -S ferronweb
```

This command installs `ferron` command, which runs a web server, and the `ferron-passwd` command, which is a password generation utility for Ferron web server. This command also installs a `systemd` service, which can be started using `sudo systemctl start ferron`.

For all `yay` prompts, press "Enter" to use the defaults.

If you haven't installed the `yay` command, you can install it using the commands below:

```bash
sudo pacman -Sy # Update the package database
sudo pacman -S pacman # Update `pacman` to fix potential dependency errors
sudo pacman -S git base-devel # Install packages required for `yay`
git clone https://aur.archlinux.org/yay-bin.git
cd yay-bin
makepkg -Cf && makepkg -si # Clean build the `yay` package, then install the package and its dependencies
```

You can view this Ferron package on [AUR](https://aur.archlinux.org/packages/ferronweb).