# ChonkOS Tools

User documentation for `chonk` and `scale`, shipped in the `chonkos-tools` package.

Both tools share the same conventions:

- Global flag `--json` prints machine-readable JSON on stdout instead of a table.
- Exit code `0` on success, `1` on any error (including failed preconditions such as missing root).
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

Prints the bundled tools documentation from `/usr/share/doc/chonkos/TOOLS.md`. With a tool name (`chonk` or `scale`), prints only that tool's section; unknown names exit `1`.

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

## Planned (v0.3)

- `nap` — suspend/hibernate helper.
- `whisker` — lightweight system information display.
- `seal` — lock down permissions or verify package signatures.
