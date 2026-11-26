# ChonkOS Architecture

## Layers

```
┌─────────────────────────────────────────────┐
│ chonkos-tools (chonk, scale) — static Rust  │
│ binaries (musl), installed via APT          │
├─────────────────────────────────────────────┤
│ chonkos-core — metapackage: base CLI tools  │
│ (git, curl, ripgrep, fd-find, fzf, tmux,    │
│ btop, zram-tools, ...)                      │
├─────────────────────────────────────────────┤
│ chonkos-branding — os-release override via  │
│ dpkg-divert, Chonky fastfetch config        │
├─────────────────────────────────────────────┤
│ APT repository — signed (GPG), hosted on   │
│ GitHub Pages, key in chonkos-archive-keyring│
├─────────────────────────────────────────────┤
│ Debian 13 (trixie) base + ZRAM (zram-tools, │
│ /dev/zram0 swap)                            │
└─────────────────────────────────────────────┘
```

## Base system

- Debian 13 (trixie), minimal install, no preinstalled bloat.
- ZRAM swap configured through `zram-tools`; the active swap device is `/dev/zram0`.

## Branding

- `chonkos-branding` ships `/usr/lib/os-release` with ChonkOS identity.
- On install/upgrade, `preinst` runs `dpkg-divert --package chonkos-branding --add --rename --divert /usr/lib/os-release.debian /usr/lib/os-release`, so the Debian original is preserved as `/usr/lib/os-release.debian` and removed on purge (`postrm`).
- Also installs the Chonky ASCII logo and a fastfetch config under `/etc/xdg/fastfetch/config.jsonc`.

## APT repository

- Packages are built as `.deb` and published to a signed APT repository on GitHub Pages.
- The repository is GPG-signed; the public key ships as `chonkos-archive-keyring.asc` and is installed to `/usr/share/keyrings/chonkos.gpg`.
- Client configuration lives in `/etc/apt/sources.list.d/chonkos.sources`.

## Tools

- `chonk` and `scale` are written in Rust and built as static musl binaries (no runtime library dependencies).
- They are packaged in `chonkos-tools` and distributed through the signed APT repository.
- Both tools support `--json` output; see [TOOLS.md](TOOLS.md).
