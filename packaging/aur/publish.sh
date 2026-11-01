#!/bin/sh
# Push packaging/aur/PKGBUILD for a release to the AUR as `lightmime-bin`. Runs on Arch (for makepkg): in release.yml
# in a container as root, or by hand for a pkgrel bump. Needs git and openssh, and the AUR account's SSH key in
# AUR_SSH_PRIVATE_KEY.
#
#   AUR_SSH_PRIVATE_KEY=... packaging/aur/publish.sh 0.4.0 dist
set -eu

version=${1:?usage: publish.sh <version> [dist dir]}
dist=${2:-}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
# Published on https://aur.archlinux.org; a keyscan that does not match is refused.
aur_ed25519=SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4

"$here/update.sh" "$version" "$dist"

cp "$here/PKGBUILD" "$work/"
if [ "$(id -u)" -eq 0 ]; then
    # makepkg refuses to run as root.
    id builder >/dev/null 2>&1 || useradd --create-home builder
    chown -R builder "$work"
    su builder -c "cd '$work' && makepkg --printsrcinfo > .SRCINFO"
else
    (cd "$work" && makepkg --printsrcinfo > .SRCINFO)
fi

printf '%s\n' "${AUR_SSH_PRIVATE_KEY:?set AUR_SSH_PRIVATE_KEY to the AUR account's SSH key}" > "$work/key"
chmod 600 "$work/key"
ssh-keyscan -t ed25519 aur.archlinux.org > "$work/known_hosts"
ssh-keygen -lf "$work/known_hosts" | grep -q " $aur_ed25519 " || {
    echo "publish.sh: aur.archlinux.org's host key does not match $aur_ed25519" >&2
    exit 1
}
export GIT_SSH_COMMAND="ssh -i $work/key -o IdentitiesOnly=yes -o UserKnownHostsFile=$work/known_hosts"

git clone ssh://aur@aur.archlinux.org/lightmime-bin.git "$work/aur"
cp "$work/PKGBUILD" "$work/.SRCINFO" "$work/aur/"
cd "$work/aur"
git add PKGBUILD .SRCINFO
if git diff --cached --quiet; then
    echo "The AUR already has lightmime-bin $version."
    exit 0
fi
git -c user.name="Pieter Willekens" -c user.email="3403851+pataar@users.noreply.github.com" \
    commit -m "Update to $version"
git push origin HEAD:master
