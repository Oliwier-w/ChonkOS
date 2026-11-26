# ChonkOS Tools

User documentation for the commands in the `chonkos-tools` package.

All five tools share these conventions.

- Global flag `--json` prints machine-readable JSON on stdout instead of a table.
- Exit code `0` indicates success. Operational failures return `1`. `nap stay` and `chonk try` return the child process's exit code.
- Set `NO_COLOR` to disable terminal colors.
- Errors are printed to stderr as a plain message (or as `{"error": "..."}` with `--json`).

## chonk

System maintenance for ChonkOS.

```
chonk [--json] <command>
```

### chonk doctor

Checks the ChonkOS release ID, Debian origin and `ID_LIKE`, the sysctl drop-in, APT sources, keyring, active `/dev/zram0` swap, root disk usage, and failed systemd units. The keyring is accepted at `/usr/share/keyrings/chonkos.gpg` or `/usr/share/keyrings/chonkos-archive-keyring.gpg`. Reports a warning when both UFW and nftables are active. The report timestamp is UTC.

```
chonk doctor
chonk --json doctor
```

Exit code is `1` if any check fails (missing os-release ID, sources, keyring, or zram; root disk usage above 85 % counts as a warning; any failed unit fails the check). The `chonkos-system` check is `warn` (not fatal) when `/etc/sysctl.d/90-chonkos.conf` is missing.

With `--json`:

```json
{"chonkos":true,"chonkos-system":"ok","apt_sources":true,"keyring":true,"dpkg_parent_debian":true,"id_like_debian":true,"zram_active":true,"root_used_percent":42,"disk_warning":false,"failed_units":"","firewall_warning":null,"timestamp":"2026-01-01T12:00:00Z"}
```

### chonk update [--yes]

Runs `apt-get update`, counts pending upgrades, then runs `apt-get upgrade` and `apt-get autoremove`. Requires root (`sudo chonk update`); exits `1` otherwise. Without `--yes`, apt prompts interactively.

```
sudo chonk update
sudo chonk update --yes
chonk --json update --yes
```

With `--json`:

```json
{"upgrades":12,"reboot_required":false}
```

### chonk clean [--dry-run] [--dev]

Without `--dry-run` (requires root): runs `apt-get clean`, `apt-get autoremove`, and `journalctl --vacuum-size=50M`. With `--dry-run`, skips those and only reports. `--dev` additionally scans `$HOME` for stale `target/` and `node_modules/` directories (no modification in dry-run; otherwise asks `y/N` per directory before removing; only deletes directories untouched for more than 30 days).

```
sudo chonk clean
sudo chonk clean --dry-run --dev
chonk --json clean --dry-run --dev
```

With `--json`:

```json
{"dry_run":true,"dev_directories":[{"path":"/home/user/proj/target","bytes":1073741824}]}
```

### chonk profile <dev|battery|homelab|status>

Changes require root and record the active profile atomically in `/var/lib/chonkos/profile`. `dev` restores normal TLP policy, suspends on lid close and turns the backlight on. `battery` selects TLP battery mode, suspends on lid close and suggests enabling the battery guard. `homelab` selects TLP AC mode, ignores lid close, and enables the guard when a battery is present. Add `--dark` to explicitly run `nap dark on` and switch off controllable backlights. It also reports LAN and Tailscale addresses. `status` shows the saved profile and checks the lid configuration, battery guard, CPU governors and ZRAM.

```
sudo chonk profile dev
sudo chonk profile battery
sudo chonk profile battery --dark
sudo chonk profile homelab
chonk profile status
```

With `--json`:

```json
{"profile":"battery","lid":"suspend","tlp_mode":"bat","guard_suggested":true,"lan":["192.168.1.10"],"tailscale":["100.64.0.2"],"dark":false}
```

### chonk card [--plain]

Shows release and kernel versions, uptime, PSS across readable processes, ZRAM usage and compression, boot time or load average, available battery and thermal readings, wildcard and loopback listeners, and the latest seal. Use `--plain` to suppress the terminal illustration and colors.

```
chonk card
chonk card --plain
chonk --json card
```

With `--json` (includes `schema_version: 1` and an RFC 3339 UTC timestamp):

```json
{"schema_version":1,"timestamp":"2026-01-01T12:00:00Z","version":"ChonkOS 0.4.0 (Debian 13 trixie)","kernel":"6.12.0","uptime_seconds":3600,"zram":{"original_bytes":1048576,"compressed_bytes":524288,"ratio":2.0},"listeners":{"wildcard":["0.0.0.0:22"],"loopback":["127.0.0.1:631"]},"latest_seal":{"id":"20260101-110000-before-upgrade","message":"before upgrade"}}
```

### chonk try [--label NAME] -- COMMAND [ARG...]

Creates a seal, runs the command synchronously, shows the changes and prints the restore preview command. The seal store must be writable by the invoking user. In JSON mode, child output goes to stderr and stdout carries the JSON result.

```
sudo chonk try --label "upgrade" -- apt-get upgrade
chonk --json try --label "tune" -- sysctl -w vm.swappiness=10
```

With `--json`:

```json
{"seal_id":"20260101-120000-upgrade","exit_code":0,"changed":["/etc/sysctl.d/90-chonkos.conf"],"restore_preview":"sudo seal restore 20260101-120000-upgrade /etc/sysctl.d/90-chonkos.conf"}
```

### chonk docs [tool]

Prints the bundled tools documentation. Supply `chonk`, `scale`, `nap`, `whisker`, or `seal` to select one section. Unknown names exit with code `1`.

```
chonk docs
chonk docs scale
```

### Shell completions

The package installs Bash, Zsh, and Fish completions for all five tools. To generate copies for custom locations, run from the `tools/` workspace:

```sh
cargo run -p xtask -- completions --out target/completions
```
## scale

Resource and size inspection. Prints a sorted table (largest first, limited to `--top` rows).

```
scale [--json] <command>
```

### scale ram [--top N]

Top processes by PSS (proportional set size, from `/proc/<pid>/smaps_rollup`), plus ZRAM statistics from `/sys/block/zram0/mm_stat`. Default `--top 10`.

```
scale ram
scale ram --top 20
scale --json ram
```

With `--json`, unit is `bytes`:

```json
{"rows":[["1234 firefox",52428800]],"unit":"bytes","zram":{"original_bytes":1048576,"compressed_bytes":524288,"ratio":2.0,"saved_bytes":524288}}
```

### scale boot [--top N]

Boot time per unit from `systemd-analyze blame`. Default `--top 10`. With `--json`, unit is `ms` and there is no `zram` key.

```
scale boot
scale --json boot --top 5
```

### scale pkg [--top N]

Installed packages by installed size (`dpkg-query -W -f='${Installed-Size}\t${Package}\n'`), tagged `[manual]` or `[auto]` based on `apt-mark showmanual`. Default `--top 15`.

```
scale pkg
scale pkg --top 30
```

### scale dir [path] [--top N]

Sizes of immediate subdirectories of `path` (default `.`). Refuses `/proc`, `/sys`, `/dev`. Default `--top 15`.

```
scale dir /var/log
scale dir . --top 5
scale --json dir /home
```

## nap

Power and lid management.

```
nap [--json] <command>
```

### nap status

Reports batteries (status, percent, health percent, watts), AC adapters (online), thermal zones (celsius), and the configured lid action read from `/etc/systemd/logind.conf.d/90-chonkos-lid.conf` (`"system default"` when the drop-in is absent).

```
nap status
nap --json status
```

### nap stay DURATION | nap stay -- COMMAND [ARG...]

Holds a `systemd-inhibit` lock (`--what=idle:sleep:handle-lid-switch --who=nap --mode=block`) so idle, suspend and lid handling are blocked. With a duration, the lock lasts that long; with `-- COMMAND`, it lasts until the command exits. Exit code is the wrapped command's exit code (or `128 + signal`); for a plain duration the exit code is `0`. With `--json`, the child's stdout is redirected to stderr and the tool prints `{"exit_code": N}`.

DURATION is a sequence of `<number><unit>` segments with unit `s`, `m` or `h` (for example `90s`, `45m`, `1h30m`); it must be positive and must not overflow.

```
nap stay 1h30m
nap stay 30m -- make -j4
nap --json stay -- systemctl restart myapp
```

### nap lid status | sleep | ignore

Reads or changes the lid switch action. `status` prints the current drop-in contents. `sleep` sets `HandleLidSwitch=suspend` and `HandleLidSwitchExternalPower=suspend`; `ignore` sets both to `ignore`. The drop-in is written atomically to `/etc/systemd/logind.conf.d/90-chonkos-lid.conf`. `sleep` and `ignore` require root and exit `1` otherwise. The tool never restarts `systemd-logind`: it only runs `systemctl reload systemd-logind` and, if that fails, reports a warning that the change takes effect after login or reboot.

```
nap lid status
sudo nap lid ignore
sudo nap lid sleep
```

### nap guard enable [--limit 60|80] | disable | status

Sets a hardware battery charge limit and persists it in a TLP drop-in. The default is 60% (`--limit 80` is also accepted).

Two hardware interfaces are supported:

- IdeaPad/Legion (`ideapad_acpi` driver): a binary `conservation_mode` switch; enabling it applies the factory limit, so both `--limit 60` and `--limit 80` map to the same conservation mode.
- ThinkPad and others (`power_supply` class): `charge_control_end_threshold` (and `charge_control_start_threshold`) are set to the requested limit.

`status` reports the detected interface, its current settings, the TLP drop-in and the maximum thermal-zone temperature. Changes require root.

```
sudo nap guard enable
sudo nap guard enable --limit 80
sudo nap guard disable
nap guard status
nap --json guard status
```

With `--json`:

```json
{"driver":"thinkpad_acpi","limit":80,"start_threshold":75,"end_threshold":80,"tlp_dropin":"/etc/tlp.d/95-chonkos-guard.conf","max_thermal_celsius":48}
```

### nap dark on | off | status

`on` switches off panel backlights, requests TTY blanking after one minute and disables Wi-Fi power saving where the available interfaces support it. It also reports the before/after battery power readings when available. `off` restores the backlights and removes the NetworkManager drop-in. `status` shows the current panel, console and drop-in state. Changes require root.

```
sudo nap dark on
sudo nap dark off
nap dark status
nap --json dark status
```

With `--json`:

```json
{"panel":"off","console_blank_minutes":1,"wifi_powersave":"off","watts_before":12.4,"watts_after":9.1}
```

## whisker

Socket, network and port inspection.

```
whisker [--json] <command>
```

### whisker ports [--all]

Lists listening sockets (TCP state `0A`, UDP state `07`) parsed from `/proc/net/tcp`, `/proc/net/tcp6`, `/proc/net/udp`, `/proc/net/udp6`, with the owning PID and process name resolved via `/proc/<pid>/fd`. Each row is tagged `exposed` (unspecified address), `local` (loopback) or `interface`. `--all` includes sockets in other states. Without `--json` a table is printed; with `--json`:

```json
{"sockets":[{"protocol":"tcp","address":"0.0.0.0:8080","state":"0A","exposure":"exposed","pid":1234,"process":"python3"}]}
```

When some processes are hidden due to permissions, a `"hint": "Use sudo to see hidden processes."` key is added.

### whisker kill PORT [--force] [--yes]

Finds the processes listening on PORT and terminates them. Sends SIGTERM by default, SIGKILL with `--force`. Without `--yes`, asks `y/N` on stderr first and aborts (exit `0`, `"terminated": false`) unless the answer is `y`. The owner is re-validated immediately before the signal is sent. Guards: PID 1, PID 0 and the tool's own PID are always protected, and OpenSSH server processes are always protected. Exits `1` if no listener is found on PORT or the owner cannot be determined (try sudo).

```
whisker kill 8080
whisker kill 8080 --yes
whisker kill 22 --force --yes
```

### whisker net [HOST]

Reports active interfaces with their addresses (via `ip -j addr show up`, falling back to `/sys/class/net`), the default gateway (from `/proc/net/route`), DNS nameservers (`/etc/resolv.conf`), the Tailscale backend state, and TCP connect latency in milliseconds to `1.1.1.1:53`.
Supply a hostname to test DNS resolution. For failed `.local` lookups, `net` and `watch` report static musl's NSS/mDNS limitation and suggest an IP address or the `systemd-resolved` stub at `127.0.0.53`.

```
whisker net
whisker --json net
```

### whisker watch HOST:PORT [--timeout SECONDS] [--interval MILLISECONDS]

Waits until a TCP connection to HOST:PORT succeeds. `--timeout` is in seconds (default `60`), `--interval` is the pause between attempts in milliseconds (default `500`, must be positive). Exit code `0` with `{"open":true,"wait_ms":N}` when the port opens, exit code `1` with `{"open":false,"wait_ms":N}` on timeout.

```
whisker watch 192.168.1.10:22
whisker watch example.com:443 --timeout 10 --interval 200
```

## seal

Configuration snapshots ("seals") of tracked system files, stored as `/var/lib/chonkos/seals/<id>/` directories containing `manifest.json`, the file copies under `files/`, and `packages.txt`.

```
seal [--json] [--store PATH] <command>
```

Tracked paths: `/etc/apt/sources.list`, `/etc/apt/sources.list.d`, `/etc/default/zramswap`, `/etc/fstab`, `/etc/systemd/logind.conf.d`, `/etc/xdg/fastfetch`, `/etc/sysctl.d`. Symlinks are skipped, and secret-looking file names (components ending in `_key`, `.pem` or `.key`, or containing `\`, `:` or NUL) are excluded from snapshots.

### seal mark MESSAGE

Creates a snapshot. The ID is `<UTC timestamp>-<slug>` where the slug is the first 48 alphanumeric characters of MESSAGE, lowercased. Each tracked file is stored with its SHA-256 in `manifest.json`; manually installed packages (`apt-mark showmanual`) are stored in `packages.txt`. With `--json` prints `{"id": "..."}`.

```
seal mark "before upgrade"
```

### seal list

Lists snapshots, newest first, as a table (`ID`, `Date`, `Description`, `Files`, `Packages`) or, with `--json`, as an array of `{"id","date","message","files","packages"}`.

### seal diff [ID]

Compares the current system with the given snapshot (default: the latest; exits `1` when there are none). With `--json`:

```json
{"id":"...","files":{"modified":["/etc/fstab"],"removed":[],"added":["/etc/sysctl.d/99-x.conf"]},"packages":{"added":["htop"],"removed":[],"modified":[]}}
```

`packages` is `null` when package tracking is unavailable.

### seal restore ID TARGET_PATH [--dry-run]

Restores a single tracked absolute path from a snapshot. TARGET_PATH must be inside the tracked set and present in the snapshot's manifest. An existing target file is first copied to `<name>.bak.<timestamp>` next to it; the restored content is checksum-verified against the manifest. `--dry-run` verifies the checksum without touching the target. Prints `{"target","backup","dry_run"}` with `--json`. Exits `1` on forbidden paths, checksum mismatch, or a changed target.

`seal restore ID --dry-run` validates every saved file and lists the targets in the snapshot.

```
sudo seal restore 20250101-120000-before-upgrade /etc/fstab --dry-run
sudo seal restore 20250101-120000-before-upgrade /etc/fstab
```

### seal export [FILE]

Writes a JSON system profile (`manual` package list, OS name/version from `/etc/os-release`, and SHA-256 of every tracked file) to FILE, or to stdout when FILE is omitted.

```
seal export
seal export /root/chonkos-profile.json
```

## Security notes

- `nap lid sleep|ignore` and `chonk profile` never restart or kill `systemd-logind`; they only reload it (`systemctl reload systemd-logind`) and warn if the reload fails.
- `whisker kill` refuses to signal PID 1, PID 0 and itself, and refuses to signal OpenSSH server processes.
- `seal` excludes traversal paths, symlinks, hard-linked files on Linux, common private-key filenames, and recognized private-key headers, and `seal restore` creates `<name>.bak.<timestamp>` backups instead of overwriting in place.
