#!/usr/bin/env bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update
cat > /etc/apt/sources.list.d/chonkos.sources <<'EOF'
Types: deb
URIs: https://oliwier-w.github.io/ChonkOS
Suites: trixie
Components: main
Architectures: amd64
Signed-By: /usr/share/keyrings/chonkos.gpg
EOF
apt-get install -y /debs/*.deb
if dpkg -s python3 >/dev/null 2>&1; then
    echo 'Unexpected python3 installation' >&2
    exit 1
fi
for field in ID=chonkos ID_LIKE=debian VERSION_CODENAME=trixie; do
    grep -Fxq "$field" /etc/os-release
done
for tool in chonk scale nap whisker seal; do
    "$tool" --version
done
# Install assertion tools after checking the dependencies of the local packages.
apt-get install -y --no-install-recommends dpkg-dev jq
dpkg-vendor --derives-from Debian
dpkg-vendor --vendor ChonkOS --derives-from Debian
status=0
chonk doctor --json >/tmp/doctor.json || status=$?
test "$status" -le 1
jq -es 'length == 1 and (.[0] | type == "object" and .chonkos == true and .apt_sources == true and .keyring == true and .dpkg_parent_debian == true and .id_like_debian == true and has("failed_units"))' /tmp/doctor.json
apt-get purge -y chonkos-core chonkos-system chonkos-shell chonkos-tools chonkos-branding chonkos-archive-keyring
apt-get autoremove -y
diversions=$(dpkg-divert --list)
if grep -qi chonkos <<<"$diversions"; then
    echo "$diversions" >&2
    exit 1
fi
grep -Fxq ID=debian /usr/lib/os-release
