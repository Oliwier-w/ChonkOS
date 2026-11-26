<p align="left">
  <img src="assets/chonky.png" width="120" alt="Chonky" />
</p>

# ChonkOS

A minimal Linux setup built on Debian 13 (trixie) for daily development and homelab use.

## Goals

- Low RAM footprint and no preinstalled bloat
- Custom CLI tools and configs maintained in one repo
- Signed APT repository for clean updates

## Install

Packages are built for `amd64` on Debian 13 (`trixie`).

```bash
curl -fsSL [https://oliwier-w.github.io/ChonkOS/install.sh](https://oliwier-w.github.io/ChonkOS/install.sh) | sudo bash
```

Preview steps without making changes:

```bash
curl -fsSL [https://oliwier-w.github.io/ChonkOS/install.sh](https://oliwier-w.github.io/ChonkOS/install.sh) | sudo bash -s -- --dry-run
```

Install with the default `nftables` firewall enabled (deny incoming, allow SSH):

```bash
curl -fsSL [https://oliwier-w.github.io/ChonkOS/install.sh](https://oliwier-w.github.io/ChonkOS/install.sh) | sudo bash -s -- --with-firewall
```

## Packages

| Package | Contents |
| :--- | :--- |
| `chonkos-archive-keyring` | GPG keyring (`/usr/share/keyrings/chonkos.gpg`); `install.sh` configures the APT source |
| `chonkos-branding` | system identity and Chonky `fastfetch` config |
| `chonkos-core` | base CLI toolset (metapackage) |
| `chonkos-system` | `systemd-zram-generator`, `sysctl`, `TLP`, and `nftables` template |
| `chonkos-shell` | POSIX profile, `bashrc`, `tmux` config, and MOTD |
| `chonkos-tools` | CLI tools (see [docs/TOOLS.md](docs/TOOLS.md)) |

`chonkos-tools` provides five static Rust binaries:

- `chonk` — system health checks, updates, cleanup, profiles, system card, and try/restore
- `scale` — memory, boot, package, and directory size inspection
- `nap` — power status, inhibit locks, lid switch, battery guard, and dark mode
- `whisker` — listening ports, process kill, and network diagnostics
- `seal` — snapshots and restore of tracked configuration files

## Status

Early work in progress (0.4).
