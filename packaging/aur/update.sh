#!/bin/sh
# Point packaging/aur/PKGBUILD at a release: set pkgver (resetting pkgrel when it changes) and fill in the checksums from the release's
# .sha256 files and the tagged source archive. Run from anywhere; edits the PKGBUILD in place.
#
#   packaging/aur/update.sh 0.4.0 [dist]
#
# With a dist directory (release.yml's downloaded artifacts) the binary checksums come from there instead of GitHub.
set -eu

version=${1:?usage: update.sh <version> [dist dir]}
dist=${2:-}
pkgbuild=$(dirname "$0")/PKGBUILD
repo=https://github.com/pataar/lightmime

binary_sum() {
    file=lightmime-$1-unknown-linux-gnu.tar.xz
    if [ -n "$dist" ]; then
        sha256sum "$dist/$file" | cut -d' ' -f1
    else
        curl -fsSL "$repo/releases/download/v$version/$file.sha256" | cut -d' ' -f1
    fi
}

source_sum=$(curl -fsSL "$repo/archive/refs/tags/v$version.tar.gz" | sha256sum | cut -d' ' -f1)
x86_64_sum=$(binary_sum x86_64)
aarch64_sum=$(binary_sum aarch64)
for sum in "$source_sum" "$x86_64_sum" "$aarch64_sum"; do
    [ ${#sum} -eq 64 ] || { echo "update.sh: bad checksum '$sum'" >&2; exit 1; }
done

# A packaging fix for the same release keeps its bumped pkgrel.
if [ "$(sed -n 's/^pkgver=//p' "$pkgbuild")" != "$version" ]; then
    sed -i "s/^pkgrel=.*/pkgrel=1/" "$pkgbuild"
fi
sed -i \
    -e "s/^pkgver=.*/pkgver=$version/" \
    -e "s/^sha256sums=.*/sha256sums=('$source_sum')/" \
    -e "s/^sha256sums_x86_64=.*/sha256sums_x86_64=('$x86_64_sum')/" \
    -e "s/^sha256sums_aarch64=.*/sha256sums_aarch64=('$aarch64_sum')/" \
    "$pkgbuild"
