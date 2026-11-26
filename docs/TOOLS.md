# ChonkOS Tools

User documentation for the commands in the `chonkos-tools` package (v0.3.0).

All five tools share these conventions.

- Global flag `--json` prints machine-readable JSON on stdout instead of a table.
- Exit code `0` indicates success. Operational failures return `1`. `nap stay` returns the child process's exit code.
- Errors are printed to stderr as a plain message (or as `{"error": "..."}` with `--json`).

## chonk

System maintenance for ChonkOS.

```
chonk [--json] <command>
```

### chonk doctor

Checks system health: ChonkOS os-release ID, APT sources, keyring, ZRAM swap, root disk usage, and failed systemd units.

```
chonk doctor
chonk --json doctor
```

Exit code is `1` if any check fails (missing os-release ID, sources, keyring, or zram; root disk usage above 85 % counts as a warning; any failed unit fails the check).

With `--json`:

```json
{"chonkos":true,"apt_sources":true,"keyring":true,"zram_active":true,"root_used_percent":42,"disk_warning":false,"failed_units":""}
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

### chonk docs [tool]

Prints the bundled tools documentation. Supply `chonk`, `scale`, `nap`, `whisker`, or `seal` to select one section. Unknown names exit with code `1`.

```
chonk docs
chonk docs scale
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

### whisker net

Reports active interfaces with their addresses (via `ip -j addr show up`, falling back to `/sys/class/net`), the default gateway (from `/proc/net/route`), DNS nameservers (`/etc/resolv.conf`), the Tailscale backend state, and TCP connect latency in milliseconds to `1.1.1.1:53`.

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

- `nap lid sleep|ignore` never restarts or kills `systemd-logind`; it only reloads it (`systemctl reload systemd-logind`) and warns if the reload fails.
- `whisker kill` refuses to signal PID 1, PID 0 and itself, and refuses to signal OpenSSH server processes.
- `seal` excludes traversal paths, symlinks, hard-linked files on Linux, common private-key filenames, and recognized private-key headers, and `seal restore` creates `<name>.bak.<timestamp>` backups instead of overwriting in place.
