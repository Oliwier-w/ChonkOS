#!/usr/bin/env bash
set -euo pipefail

# Parse the entire installation before executing any command from a pipe.
{
REPO_URL="https://oliwier-w.github.io/ChonkOS"
KEYRING_PATH="/usr/share/keyrings/chonkos.gpg"
SOURCES_PATH="/etc/apt/sources.list.d/chonkos.sources"
dry=false
force=false
firewall=false
for arg in "$@"; do
    case "$arg" in
        --dry-run) dry=true ;;
        --force) force=true ;;
        --with-ufw|--with-firewall) firewall=true ;;
        -h|--help) echo 'Usage: install.sh [--dry-run] [--force] [--with-firewall]'; exit 0 ;;
        *) echo "Unknown option: $arg" >&2; exit 2 ;;
    esac
done
if [ "$dry" != true ] && (( EUID != 0 )); then
    echo 'Run as root or use --dry-run.' >&2
    exit 1
fi
if [ "$force" != true ]; then
    # shellcheck source=/dev/null
    if [[ -r /etc/os-release ]]; then . /etc/os-release; fi
    if [[ ${ID:-} != chonkos && ( ${ID:-} != debian || ${VERSION_CODENAME:-} != trixie ) ]]; then
        echo 'Expected Debian trixie or ChonkOS. Use --force to override.' >&2
        exit 1
    fi
    if [ "$(dpkg --print-architecture)" != amd64 ]; then
        echo 'ChonkOS packages are built for amd64 only. Use --force to override.' >&2
        exit 1
    fi
fi
if [ "$dry" = true ]; then
    echo "Would install $KEYRING_PATH, write $SOURCES_PATH, update APT, install chonkos-core and run chonk doctor."
    if [ "$firewall" = true ]; then echo 'Would allow OpenSSH and available tailscale0 before enabling UFW.'; fi
    exit 0
fi
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
curl -fsSL --proto '=https' --tlsv1.2 --retry 3 --connect-timeout 15 \
    "$REPO_URL/chonkos-archive-keyring.asc" -o "$tmp/key.asc"
gpg --batch --dearmor --output "$tmp/key.gpg" "$tmp/key.asc"
(cd "$tmp" && printf '%s  key.gpg\n' '14e1ad3fe4d8e5c60f2b961f8583e4bd175f50768b0a3b14b4c242084062edf4' | sha256sum --check --status) || {
    echo 'Unexpected ChonkOS repository key.' >&2
    exit 1
}
install -D -m 0644 "$tmp/key.gpg" "$KEYRING_PATH"
# The keyring package does not own APT source configuration; install.sh manages it.
cat > "$tmp/chonkos.sources" <<EOF
Types: deb
URIs: $REPO_URL
Suites: trixie
Components: main
Architectures: amd64
Signed-By: $KEYRING_PATH
EOF
install -D -m 0644 "$tmp/chonkos.sources" "$SOURCES_PATH"
if ! { apt-get update && apt-get install -y chonkos-core; }; then
    rm -f "$SOURCES_PATH"
    echo 'Installation failed; the ChonkOS APT source was removed.' >&2
    exit 1
fi
if [ "$firewall" = true ]; then
    apt-get install -y ufw
    ssh_ports=22
    if command -v sshd >/dev/null 2>&1; then
        detected=$(sshd -T 2>/dev/null | awk '$1 == "port" { print $2 }' | sort -u)
        if [ -n "$detected" ]; then ssh_ports=$detected; fi
    fi
    printf '%s\n' "$ssh_ports" | while IFS= read -r port; do
        if [ -n "$port" ]; then ufw allow "$port/tcp"; fi
    done
    ufw default deny incoming
    ufw default allow outgoing
    if [[ -e /sys/class/net/tailscale0 ]]; then ufw allow in on tailscale0; fi
    if ! ufw --force enable; then
        echo 'Failed to enable UFW; the firewall rules were not activated.' >&2
        exit 1
    fi
    echo 'UFW allows only OpenSSH and Tailscale; other inbound services are now blocked.' >&2
fi
chonk doctor || echo 'chonk doctor reported a problem; run it manually for details.' >&2
echo 'ChonkOS installed. Log in again to load chonkos-shell.'
} </dev/null
