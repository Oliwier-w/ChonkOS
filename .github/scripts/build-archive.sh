#!/usr/bin/env bash
set -euo pipefail
# Run from the source checkout. pages/ is the checked-out gh-pages worktree.
source_dir=$PWD
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd pages
mkdir -p pool/main
for deb in "$source_dir"/build/*.deb; do
    dest=pool/main/${deb##*/}
    if [[ -e $dest ]]; then
        cmp "$deb" "$dest" || { echo "Published package changed: $dest; bump its version" >&2; exit 1; }
    else
        cp "$deb" "$dest"
    fi
done
cp "$source_dir/scripts/install.sh" "$source_dir/build/metrics.json" .
cp "$source_dir"/build/chonkos-archive-keyring_*_all.deb chonkos-archive-keyring.deb
touch .nojekyll

apt-ftparchive packages pool/main > "$work/all-packages"
suite=dists/trixie
for arch in amd64 all; do
    dst="$work/indices/main/binary-$arch"
    mkdir -p "$dst"
    awk -v arch="$arch" 'BEGIN { RS=""; ORS="\n\n" }
        $0 ~ ("(^|\n)Architecture: " arch "(\n|$)") { print }' "$work/all-packages" > "$dst/Packages"
    test -s "$dst/Packages"
    gzip -n -c "$dst/Packages" > "$dst/Packages.gz"
    xz -c "$dst/Packages" > "$dst/Packages.xz"
done
# Generate Release in isolation so it contains exactly the six current indices.
apt-ftparchive --no-md5 --no-sha1 --no-sha512 \
    -o APT::FTPArchive::Release::Origin=ChonkOS \
    -o APT::FTPArchive::Release::Label=ChonkOS \
    -o APT::FTPArchive::Release::Suite=trixie \
    -o APT::FTPArchive::Release::Codename=trixie \
    -o 'APT::FTPArchive::Release::Architectures=amd64 all' \
    -o APT::FTPArchive::Release::Components=main \
    -o APT::FTPArchive::Release::Acquire-By-Hash=yes \
    release "$work/indices" > "$work/Release"
grep -Fxq 'Acquire-By-Hash: yes' "$work/Release"
mkdir -p "$suite"
cp -R "$work/indices/." "$suite/"
for index in "$suite"/main/binary-{amd64,all}/Packages{,.gz,.xz}; do
    hash=$(sha256sum "$index" | cut -d' ' -f1)
    awk -v digest="$hash" -v path="${index#"$suite/"}" \
        '$1 == digest && $3 == path { found=1 } END { exit !found }' "$work/Release"
    blob=${index%/*}/by-hash/SHA256/$hash
    mkdir -p "${blob%/*}"
    cp "$index" "$blob"
    echo "$blob" >> "$work/generation"
done
history=.by-hash-generations
mkdir -p "$history"
if ! cmp -s "$work/generation" "$history/current"; then
    if [[ -f $history/previous ]]; then cp "$history/previous" "$history/oldest"; fi
    if [[ -f $history/current ]]; then cp "$history/current" "$history/previous"; fi
    cp "$work/generation" "$history/current"
fi
cat "$history"/* > "$work/keep"
for blob in "$suite"/main/binary-{amd64,all}/by-hash/SHA256/*; do
    if ! grep -Fxq "$blob" "$work/keep"; then rm -- "$blob"; fi
done
cp "$work/Release" "$suite/Release"
export GNUPGHOME="$work/gnupg"
mkdir -m 700 "$GNUPGHOME"
printf '%s\n' "$GPG_PRIVATE_KEY" | gpg --batch --import
key=$(gpg --show-keys --with-colons "$source_dir/chonkos-archive-keyring.asc" | awk -F: '$1=="fpr" && !found {print $10; found=1}')
test -n "$key"
gpg --batch --export "$key" > chonkos-archive-keyring.gpg
cmp chonkos-archive-keyring.gpg "$source_dir/packages/chonkos-archive-keyring/usr/share/keyrings/chonkos.gpg"
gpg --batch --armor --export "$key" > chonkos-archive-keyring.asc
sign=(gpg --batch --yes --pinentry-mode loopback --passphrase '' --local-user "$key" --digest-algo SHA256)
"${sign[@]}" --armor --detach-sign -o "$suite/Release.gpg" "$suite/Release"
"${sign[@]}" --clearsign -o "$suite/InRelease" "$suite/Release"
gpgv --keyring "$PWD/chonkos-archive-keyring.gpg" "$suite/InRelease"
sha256sum pool/main/*.deb install.sh chonkos-archive-keyring.{deb,asc,gpg} metrics.json > SHA256SUMS
