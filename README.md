<p align="left">
  <img src="assets/chonky.png" width="120" alt="Chonky" />
</p>

# ChonkOS

A minimal Linux setup built on Debian 13 (trixie) for daily development and homelab use.

## Goals
- Low RAM footprint and no preinstalled bloat
- Custom CLI tools and configs maintained in one repo
- Signed APT repository for clean updates

## Packages
- `chonkos-branding` — system identity and Chonky fastfetch config
- `chonkos-core` — base CLI toolset
- `chonkos-tools` — CLI tools (see [docs/TOOLS.md](docs/TOOLS.md)):
  - `chonk` — system health checks, updates and cleanup
  - `scale` — memory, boot, package and directory size inspection
  - `nap` — power status, inhibit locks and lid switch control
  - `whisker` — listening ports, process kill and network diagnostics
  - `seal` — snapshots and restore of tracked configuration files

## Status
Early work in progress (0.3).
