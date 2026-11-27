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
curl -fsSL https://oliwier-w.github.io/ChonkOS/install.sh | sudo bash
```

Preview steps without making changes:

```bash
curl -fsSL https://oliwier-w.github.io/ChonkOS/install.sh | sudo bash -s -- --dry-run
```

Install with the default `nftables` firewall enabled (deny incoming, allow SSH):

```bash
curl -fsSL https://oliwier-w.github.io/ChonkOS/install.sh | sudo bash -s -- --with-firewall
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
| `chonkos-desktop` | Arctic Wayland desktop: Sway, Waybar, Foot, Fuzzel, Mako, greetd + tuigreet |
| `chonkos-desktop-x11` | optional X11 fallback (i3 + Dunst) for legacy NVIDIA GPUs; conflicts with `chonkos-desktop` |

`chonkos-tools` provides five static Rust binaries:

- `chonk` — system health checks, updates, cleanup, profiles, modes, theming, system card, and try/restore
- `scale` — memory, boot, package, and directory size inspection
- `nap` — power status, inhibit locks, lid switch, battery guard, dark mode, and UPS shutdown
- `whisker` — listening ports, expiring firewall leases, drift detection, process kill, and network diagnostics
- `seal` — snapshots and restore of tracked configuration files

## Editions and Chameleon Mode

- `chonkos-core` is the headless base: CLI tools, system tuning, and shell configuration.
- `chonkos-desktop` adds the flat Arctic Wayland desktop (Sway + Waybar + Foot + Fuzzel + Mako) with a `tuigreet` graphical login on VT 7.
- `chonkos-desktop-x11` is the optional i3 + Dunst fallback for NVIDIA driver series 550 and older, where Sway cannot start. The two desktop packages conflict and cannot be co-installed.

Chameleon Mode switches the machine between the two personalities:

```bash
sudo chonk mode desktop   # graphical.target + greetd login
sudo chonk mode homelab   # multi-user.target + TTY1 status dashboard (chonkos-tty-dash@tty1)
chonk mode status
```

Theming is driven by `~/.config/chonkos/chonk.toml` and materialized by `chonk apply` into `~/.config/chonkos/generated/`, which the session wrappers layer over the shipped defaults in `/usr/share/chonkos/`:

```toml
schema_version = 1

[system]
mode = "desktop"
profile = "dev"

[power]
battery_guard = "80"
lid_homelab = "ignore"
dive_shutdown_percent = 20

[appearance]
palette = "arctic-slate"
font_mono = "JetBrains Mono"
font_size = 11
```

## Status

Early work in progress (0.5).
