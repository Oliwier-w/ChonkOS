# ChonkOS Architecture

## Layers

```
┌─────────────────────────────────────────────┐
│ chonkos-tools (chonk, scale, nap, whisker,  │
│ seal) — five static Rust binaries (musl,    │
│ < 1 MB each) via APT                        │
├─────────────────────────────────────────────┤
│ chonkos-shell — /etc/profile.d/chonkos.sh,  │
│ /etc/tmux.conf, MOTD (update-motd.d)        │
├─────────────────────────────────────────────┤
│ chonkos-system — zram-tools drop-in,        │
│ sysctl.d, tlp.d, ufw                        │
├─────────────────────────────────────────────┤
│ chonkos-core — metapackage: base CLI tools  │
│ (git, curl, ripgrep, fd-find, fzf, tmux,    │
│ btop, ...)                                  │
├─────────────────────────────────────────────┤
│ chonkos-branding — os-release override,     │
│ Chonky fastfetch config                     │
├─────────────────────────────────────────────┤
│ chonkos-archive-keyring — GPG keyring for   │
│ the signed APT repository                   │
├─────────────────────────────────────────────┤
│ APT repository — signed (GPG), hosted on   │
│ GitHub Pages with Acquire-By-Hash, +        │
│ install.sh one-liner installer              │
├─────────────────────────────────────────────┤
│ Debian 13 (trixie) base + ZRAM swap via     │
│ zram-tools (/dev/zram0)                     │
└─────────────────────────────────────────────┘
```

## Base system

- Debian 13 (trixie), minimal install, no preinstalled bloat, no Python in the system layer, no telemetry.
- ZRAM swap is provided by `zram-tools`; the active swap device is `/dev/zram0`.

## System tuning (chonkos-system)

- Ships `/etc/sysctl.d/90-chonkos.conf` (swappiness, page-cluster, vfs cache pressure, dirty ratios) and `/etc/tlp.d/90-chonkos.conf` (governors, Wi-Fi power, runtime PM).
- ZRAM is provided by `zram-tools`: the package ships `etc/chonkos/zramswap` (zstd, 50 %, priority 100) and a `zramswap.service.d` drop-in that bind-mounts it read-only over `/etc/default/zramswap`. The distribution conffile is therefore never modified and nothing has to be restored on purge.
- The firewall uses UFW: `install.sh --with-firewall` (alias for `--with-ufw`) installs `ufw`, allows every port configured in `sshd -T` plus the tailscale0 interface, and enables the default deny-incoming policy. `chonk doctor` warns when both UFW and nftables are active at the same time.
- `postinst` applies sysctls (`sysctl --system`) and enables/restarts `tlp.service` and `zramswap.service` when systemd is running; a failing unit produces a warning instead of aborting the configure step.

## Shell environment (chonkos-shell)

- `/etc/profile.d/chonkos.sh` — interactive Bash prompt with git branch, `fd`/`ll`/`la`/`gs`/`gl` aliases and fzf key bindings (Debian's `fd-find` is aliased to `fd`).
- `/etc/tmux.conf` — mouse on, base index 1, renumbering, 10k history, minimal status bar.
- `/etc/update-motd.d/90-chonkos` — login MOTD line with OS, kernel, uptime, RAM usage and `/dev/zram0` state, plus a pointer to `chonk doctor`.

## Branding

- `chonkos-branding` ships `/usr/lib/os-release` with ChonkOS identity.
- On install/upgrade, `preinst` runs `dpkg-divert --package chonkos-branding --add --no-rename --divert /usr/lib/os-release.debian /usr/lib/os-release`. `--no-rename` is required because `/usr/lib/os-release` belongs to the Essential `base-files` package; a copy of the Debian file is kept as `/usr/lib/os-release.debian` and restored on removal (`postrm`).
- Also installs the Chonky ASCII logo and a fastfetch config under `/etc/xdg/fastfetch/config.jsonc`.

## APT repository

- Packages are built as `.deb` and published to a signed APT repository on GitHub Pages.
- The repository is GPG-signed; the public key ships in `chonkos-archive-keyring` (repo file `chonkos-archive-keyring.asc`) and is installed to `/usr/share/keyrings/chonkos.gpg`.
- Client configuration lives in `/etc/apt/sources.list.d/chonkos.sources`.
- Indexes are built with `Acquire-By-Hash=yes` (`APT::FTPArchive::Release::Acquire-By-Hash`), so every `Packages`/`Sources` index is also published under `by-hash/SHA256/<digest>` on GitHub Pages. Clients can therefore fetch indexes by content hash, which makes fetches atomic and cache-friendly; the Release file is verified to actually carry `Acquire-By-Hash: yes` during the build.
- PRs and pushes to `main` and `release/*` build and test the Debian packages. Version tags and manual releases run the same checks and publish the tested artifacts. Configure Pages to serve the root of `gh-pages`.
- Publication retains versioned packages and the last three distinct index generations by SHA256. The archive rejects replacement of an existing package filename with different bytes. A manual release uses the version from `metrics.json`.

## Installer

- `scripts/install.sh` is copied to the repository root and served on GitHub Pages as `https://oliwier-w.github.io/ChonkOS/install.sh`.
- It installs the keyring to `/usr/share/keyrings/chonkos.gpg`, writes a sources file byte-identical to the `chonkos-archive-keyring` conffile, installs `chonkos-core` (which depends on the keyring package), and runs `chonk doctor`. `--dry-run` prints the planned actions without changes; `--with-firewall` (alias `--with-ufw`) additionally installs UFW and enables it, allowing the ports reported by `sshd -T` and the tailscale0 interface.

## Version SSOT (tools/xtask)

- The workspace version `0.4.0` in `tools/Cargo.toml` is the single source of truth (SSOT) for all five binaries and the Debian package versions.
- `cargo run -p xtask` reads that version and propagates it to generated artifacts (completions, packaging metadata), so the version is bumped in exactly one place.

## Tools and musl DNS limitation

- `chonk`, `scale`, `nap`, `whisker` and `seal` are written in Rust and built as five static musl binaries (no runtime library dependencies, each under 1 MB).
- They are packaged in `chonkos-tools` and distributed through the signed APT repository; the package also installs shell completions for `chonk` automatically.
- All five tools support `--json` output; see [TOOLS.md](TOOLS.md).
- Because the binaries are statically linked against musl, glibc NSS modules are unavailable: mDNS/`.local` resolution does not work in the tools themselves. `whisker net` and `whisker watch` detect failed `.local` lookups and report this limitation, suggesting a direct IP address or the `systemd-resolved` stub at `127.0.0.53`.
- `seal` stores its configuration snapshots under `/var/lib/chonkos/seals/`; `chonk profile` stores the active profile in `/var/lib/chonkos/profile`.
