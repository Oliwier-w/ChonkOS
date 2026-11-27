#!/usr/bin/env bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
core=(chonkos-core chonkos-system chonkos-shell chonkos-tools chonkos-branding chonkos-archive-keyring)
debs=()
for package in "${core[@]}"; do
    files=(/debs/"${package}"_*.deb)
    test "${#files[@]}" -eq 1
    test -f "${files[0]}"
    debs+=("${files[0]}")
done
apt-get update
purge() {
    apt-get purge -y "$@" "${core[@]}"
    apt-get autoremove --purge -y
    local diversions
    diversions=$(dpkg-divert --list)
    if grep -qi chonkos <<<"$diversions"; then
        echo "$diversions" >&2
        exit 1
    fi
    grep -Fxq ID=debian /etc/os-release
}

case "${1:-}" in
core)
    # The normal installer provisions this source. These tests use local packages.
    cat > /etc/apt/sources.list.d/chonkos.sources <<'EOF'
Types: deb
URIs: https://oliwier-w.github.io/ChonkOS
Suites: trixie
Components: main
Architectures: amd64
Signed-By: /usr/share/keyrings/chonkos.gpg
Enabled: no
EOF
    apt-get install -y --no-install-recommends "${debs[@]}"
    ! dpkg -s python3 >/dev/null 2>&1 || { echo 'Unexpected python3 installation' >&2; exit 1; }
    ! dpkg -s perl >/dev/null 2>&1 || { echo 'Unexpected perl installation' >&2; exit 1; }
    for field in ID=chonkos ID_LIKE=debian VERSION_CODENAME=trixie; do
        grep -Fxq "$field" /etc/os-release
    done
    for tool in chonk scale nap whisker seal; do
        "$tool" --version
    done
    apt-get install -y --no-install-recommends dpkg-dev jq nftables
    dpkg-vendor --derives-from Debian
    dpkg-vendor --vendor ChonkOS --derives-from Debian
    # A container can lack host services and swap, which doctor reports with exit 1.
    status=0
    chonk doctor --json >/tmp/doctor.json || status=$?
    test "$status" -le 1
    jq -es 'length == 1 and (.[0] | .chonkos and .apt_sources and .keyring and .dpkg_parent_debian and .id_like_debian and has("failed_units"))' /tmp/doctor.json
    chonk mood --waybar > /tmp/mood.json
    test "$(wc -l < /tmp/mood.json)" -eq 1
    jq -es 'length == 1 and (.[0] | .text == "CHONK" and (.tooltip | type == "string") and (.alt as $state | .class == ["chonky", "state-\($state)"]))' /tmp/mood.json
    chonk card --json | jq -e '.schema_version == 1 and has("timestamp")'
    whisker drift --json | jq -e '.status == "uninitialized"'
    nap dive status --json | jq -e '.status == "disabled"'
    rules=/usr/share/chonkos/nftables/default.nft
    for proto in tcp udp; do
        grep -Eq "^[[:space:]]*set chonkos_leases_${proto}[[:space:]]*[{]" "$rules"
    done
    mkdir -p /etc/nftables.d
    if nft list tables >/tmp/nft-probe.log 2>&1; then
        nft -c -f "$rules"
    elif grep -Eqi 'operation not permitted|permission denied|protocol not supported|operation not supported' /tmp/nft-probe.log; then
        cat /tmp/nft-probe.log
        echo 'Skipping nftables kernel validation in this container'
    else
        cat /tmp/nft-probe.log >&2
        exit 1
    fi
    HOME=$(mktemp -d)
    export HOME
    unset XDG_CONFIG_HOME
    config=$HOME/.config/chonkos
    override=$config/overrides/sway/99-test.conf
    mkdir -p "${override%/*}"
    printf '# Preserve this override\n' > "$override"
    sha256sum "$override" >/tmp/override.sha256
    chonk apply --dry-run --json | jq -e '.dry_run and (.files | length == 8)'
    test ! -e "$config/generated"
    chonk apply --json | jq -e '(.dry_run | not) and (.files | length == 8)'
    test -f "$config/generated/sway-colors.conf"
    sha256sum -c /tmp/override.sha256
    purge
    ;;
desktop)
    apt-get install -y "${debs[@]}" /debs/chonkos-desktop_*.deb
    dpkg-query -S /etc/greetd/config.toml | grep -Fx 'greetd: /etc/greetd/config.toml'
    for file in /etc/greetd/chonkos.toml /usr/lib/tmpfiles.d/chonkos-tuigreet.conf /usr/lib/systemd/system/greetd.service.d/90-chonkos.conf; do
        test -f "$file"
    done
    grep -F 'ChonkOS ' /etc/greetd/chonkos.toml
    for wrapper in sway-session waybar-launch foot-launch fuzzel-launch mako-launch; do
        test -x "/usr/libexec/chonkos/$wrapper"
    done
    /var/lib/dpkg/info/chonkos-desktop.postinst configure
    purge chonkos-desktop
    ;;
x11)
    apt-get install --dry-run "${debs[@]}" /debs/chonkos-desktop-x11_*.deb
    ;;
*)
    echo 'Expected core, desktop, or x11' >&2
    exit 2
    ;;
esac
