# Ferron Linux installer

Self-extracting shell installer for Linux. It's run as a script in `dist/install.sh` (after building) that boots the installer when run as root.

## Building

```
just installer   # runs `make` in installer/
# or, manually:
make -C installer
```

## Running

```
sudo ./dist/install.sh
```

See the [Ferron documentation](https://ferron.sh/docs/installation/linux/installer) for usage.
