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
│ chonkos-desktop — Sway + Waybar + Foot +    │
│ Fuzzel + Mako, greetd/tuigreet (Wayland)    │
│ chonkos-desktop-x11 — i3 + Dunst fallback   │
├─────────────────────────────────────────────┤
│ chonkos-system — zram-tools drop-in,        │
│ sysctl.d, tlp.d, nftables template, udev    │
│ dive hook, TTY dashboard unit               │
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
- The base firewall ships as `/usr/share/chonkos/nftables/default.nft` (table `inet chonkos_filter`, default-drop input chain) and ends with `include "/etc/nftables.d/*.nft"`. `whisker lease add` inserts ports into the native `flags timeout` sets `chonkos_leases_tcp` / `chonkos_leases_udp`; the kernel expires the entries by itself, so leases fail closed on reboot. `install.sh --with-firewall` still installs UFW; `chonk doctor` warns when both UFW and nftables are active. `postinst` never enables `nftables.service`.
- Battery (UPS) events reach userspace via udev: `90-chonkos-dive.rules` reacts to `power_supply` `change` events by setting `SYSTEMD_WANTS=chonkos-nap-dive.service` (never `RUN+=`). The oneshot runs `/usr/bin/nap dive --event` only while `/var/lib/chonkos/nap-dive.enabled` exists. `postinst` reloads udev rules with `udevadm control --reload-rules` only when systemd is running.
- `postinst` applies sysctls (`sysctl --system`) and enables/restarts `tlp.service` and `zramswap.service` when systemd is running; a failing unit produces a warning instead of aborting the configure step.

## Shell environment (chonkos-shell)

- `/etc/profile.d/chonkos.sh` — interactive Bash prompt with git branch, `fd`/`ll`/`la`/`gs`/`gl` aliases and fzf key bindings (Debian's `fd-find` is aliased to `fd`).
- `/etc/tmux.conf` — mouse on, base index 1, renumbering, 10k history, minimal status bar.
- `/etc/update-motd.d/90-chonkos` — login MOTD line with OS, kernel, uptime, RAM usage and `/dev/zram0` state, plus a pointer to `chonk doctor`.

## Zero-Clobber configuration layers

All ChonkOS-managed configuration follows four layers, and package upgrades never overwrite user files:

1. `/usr/share/chonkos/` — shipped defaults (Layer 0).
2. `~/.config/chonkos/chonk.toml` — user intent (Layer 1).
3. `~/.config/chonkos/generated/` — files written by `chonk apply` from `chonk.toml` (Layer 2).
4. `~/.config/chonkos/overrides/` — hand-written user files (Layer 3), always winning.

The wrappers in `/usr/libexec/chonkos/` assemble runtime configs from these layers at session start. `sway-session` uses segment shadowing: for each `/usr/share/chonkos/sway/conf.d/*.conf`, an override with the same basename replaces the whole shipped segment (so two `bar {}` blocks can never collide), and non-shadowing overrides are appended last. `waybar-launch`, `foot-launch` and `fuzzel-launch` emit `@import`/`include=` lines only for files that actually exist; `mako-launch` concatenates the existing layers because mako has no `include`. `/etc/tmux.conf` sources `generated/tmux-theme.conf` and `overrides/tmux/tmux.conf` when readable.

## Desktop and greeter (chonkos-desktop, chonkos-desktop-x11)

- `chonkos-desktop` ships the Sway/Wayland session; `chonkos-desktop-x11` ships i3 + Dunst for NVIDIA driver series 550 and older (the `sway-session` wrapper exits with a recommendation to install it). The two packages `Conflicts:` each other.
- greetd integration never touches `/etc/greetd/config.toml`, which is owned by Debian's `greetd` package. Each edition ships its own conffile (`/etc/greetd/chonkos.toml` or `/etc/greetd/chonkos-x11.toml`, rendered from a `.in` template) and a `greetd.service.d` drop-in that resets `ExecStart=` and points greetd at that file with `--config`.
- The greeter runs as the `_greetd` system user on VT 7 via `tuigreet`. `/usr/lib/tmpfiles.d/chonkos-tuigreet.conf` creates `/var/cache/tuigreet`; `postinst` applies it with `systemd-tmpfiles --create` only when systemd is running. Neither desktop package starts greetd during install — `chonk mode desktop` enables it.
- In homelab mode, `chonk mode homelab` instead enables `chonkos-tty-dash@tty1.service`, a passive dashboard that runs `/usr/libexec/chonkos/tty-dash-loop` (refreshing `chonk card --tty` every 15 s) on TTY1 and conflicts with `getty@tty1`.

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
