#!/usr/bin/env bash
# Build lightmime's Flatpak bundle, on the `stable` branch, from a release binary.
#
# Usage: packaging/linux/flatpak.sh BINARY ARCH OUTPUT
#   ARCH is x86_64 or aarch64, and must be this machine's. Needs flatpak-builder (or $FLATPAK_BUILDER, e.g.
#   "flatpak run org.flatpak.Builder") and the flathub remote, from which it installs the Freedesktop runtime and SDK
#   the manifest names, and rsvg-convert for the icons. $RELEASE_DATE (YYYY-MM-DD, default today) dates the release in the metainfo.
set -euo pipefail

binary=${1:?usage: flatpak.sh BINARY ARCH OUTPUT}
arch=${2:?usage: flatpak.sh BINARY ARCH OUTPUT}
output=${3:?usage: flatpak.sh BINARY ARCH OUTPUT}
app_id=io.github.pataar.lightmime
flathub=https://dl.flathub.org/repo/flathub.flatpakrepo
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
read -ra builder <<<"${FLATPAK_BUILDER:-flatpak-builder}"
version=$("$binary" --version | awk '{ print $2 }')
date=${RELEASE_DATE:-$(date -u +%F)}

# Under the repo's target/ rather than /tmp: a sandboxed builder (org.flatpak.Builder) has its own /tmp.
mkdir -p "$here/../../target"
work=$(mktemp -d "$here/../../target/flatpak.XXXXXX")
trap 'rm -rf "$work"' EXIT

install -Dm755 "$binary" "$work/stage/lightmime"
install -Dm755 "$here/flatpak/lightmime-launch" "$work/stage/lightmime-launch"
# Native installs use the plain icon name; a Flatpak may only export icons named after its app id.
sed -e "s/^Icon=lightmime$/Icon=$app_id/" "$here/../../contrib/$app_id.desktop" >"$work/stage/$app_id.desktop"
# Software centres show the version from the metainfo's releases, and the screenshots of that release.
sed -e "s|/lightmime/main/|/lightmime/v$version/|" -e "s|^</component>|  <releases>\n    <release version=\"$version\" date=\"$date\"/>\n  </releases>\n</component>|" \
    "$here/$app_id.metainfo.xml" >"$work/stage/$app_id.metainfo.xml"
"$here/../../assets/brand/export.sh" "$work/icons"
while read -r icon; do
    name=$(basename "$icon")
    install -Dm644 "$icon" "$work/stage/$(dirname "${icon#"$work/icons/"}")/$app_id${name#lightmime}"
done < <(find "$work/icons/hicolor" -type f)
cp "$here/flatpak/$app_id.yml" "$work/"

flatpak remote-add --user --if-not-exists flathub "$flathub"
# --disable-rofiles-fuse: CI runners have no FUSE.
"${builder[@]}" --arch="$arch" --user --install-deps-from=flathub --disable-rofiles-fuse --force-clean \
    --default-branch=stable --repo="$work/repo" --state-dir="$work/state" "$work/build" "$work/$app_id.yml"
# The bundle names flathub as the runtime's source, so `flatpak install` fetches the runtime from there.
flatpak build-bundle --arch="$arch" --runtime-repo="$flathub" "$work/repo" "$output" "$app_id" stable
