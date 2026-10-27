#!/usr/bin/env bash
# Add lightmime's Flatpak repository to the docs site, so `flatpak update` brings new releases.
#
# Usage: packaging/linux/flatpak-repo.sh SITE
#   SITE is the built site (docs/book); this writes SITE/flatpak/repo and SITE/flatpak/lightmime.flatpakrepo. Every
#   Pages deploy replaces the whole site, so the repository is mirrored from the live site first, then the latest
#   release's bundles are imported into it. Needs flatpak, ostree, curl and gh (with GH_TOKEN and GITHUB_REPOSITORY);
#   $SITE_URL is the site's address. Before the first release with bundles, it writes nothing. A failure to read the
#   live repository or the release stops the script, so a deploy never drops the repository. With
#   $FLATPAK_GPG_PRIVATE_KEY (an armored secret key without a passphrase) the commits and summary are signed and the
#   .flatpakrepo carries the public key, so `flatpak remote-add` verifies them, and the mirror of the live site must be
#   signed by that key too, so a site serving other commits stops the deploy; without it the repository is unsigned.
set -euo pipefail

site=${1:?usage: flatpak-repo.sh SITE}
app_id=io.github.pataar.lightmime
base_url=${SITE_URL:?SITE_URL must be set}
base_url=${base_url%/}
repo=$site/flatpak/repo
bundles=$(mktemp -d)
trap 'rm -rf "$bundles"' EXIT

mkdir -p "$repo"
ostree --repo="$repo" init --mode=archive-z2

sign=()
gpg_key_line=
# With the key, the mirror must carry its signature: the key never signs what the live site serves unchecked.
verify=(--no-gpg-verify)
if [ -n "${FLATPAK_GPG_PRIVATE_KEY:-}" ]; then
    gnupg=$(mktemp -d)
    trap 'rm -rf "$bundles" "$gnupg"' EXIT
    gpg --homedir "$gnupg" --batch --quiet --import <<<"$FLATPAK_GPG_PRIVATE_KEY"
    key=$(gpg --homedir "$gnupg" --batch --with-colons --list-secret-keys | awk -F: '$1 == "fpr" { print $10; exit }')
    sign=(--gpg-sign="$key" --gpg-homedir="$gnupg")
    gpg --homedir "$gnupg" --batch --export "$key" >"$gnupg/public.gpg"
    gpg_key_line="GPGKey=$(base64 -w0 <"$gnupg/public.gpg")"
    verify=(--set=gpg-verify=true --set=gpg-verify-summary=true --gpg-import="$gnupg/public.gpg")
else
    echo "FLATPAK_GPG_PRIVATE_KEY is not set; the repository is unsigned"
fi

status=$(curl -sSL -o /dev/null -w '%{http_code}' "$base_url/flatpak/repo/summary")
case $status in
    200)
        ostree --repo="$repo" remote add "${verify[@]}" live "$base_url/flatpak/repo"
        ostree --repo="$repo" pull --mirror live
        ostree --repo="$repo" remote delete live
        ;;
    404) ;;
    *)
        echo "reading the live Flatpak repository failed with HTTP $status" >&2
        exit 1
        ;;
esac

tag=$(gh release view --repo "$GITHUB_REPOSITORY" --json tagName --jq .tagName)
# A release that flatpak.yml has not attached bundles to yet has none to download.
assets=$(gh release view "$tag" --repo "$GITHUB_REPOSITORY" --json assets --jq '.assets[].name')
if grep -q '^lightmime-.*\.flatpak$' <<<"$assets"; then
    gh release download "$tag" --repo "$GITHUB_REPOSITORY" --pattern 'lightmime-*.flatpak' --dir "$bundles"
fi
for bundle in "$bundles"/lightmime-*.flatpak; do
    [ -e "$bundle" ] || continue
    echo "importing $(basename "$bundle") from $tag"
    flatpak build-import-bundle "$repo" "$bundle"
done

if [ -z "$(ostree --repo="$repo" refs)" ]; then
    echo "no Flatpak release yet; the site gets no repository"
    rm -rf "$site/flatpak"
    exit 0
fi
if [ ${#sign[@]} -gt 0 ]; then
    # Signs every ref's tip: the imported ones, and mirrored ones, which the pull verified already carry this key's
    # signature; ostree refuses to sign a commit twice, so that refusal for this key is not a failure.
    while read -r ref; do
        if ! output=$(ostree --repo="$repo" gpg-sign --gpg-homedir="$gnupg" \
            "$(ostree --repo="$repo" rev-parse "$ref")" "$key" 2>&1); then
            grep -q "already signed with GPG key $key" <<<"$output" || { echo "$output" >&2; exit 1; }
        fi
    done < <(ostree --repo="$repo" refs)
fi
# Each imported bundle is a root commit and the mirror copies only each ref's tip, so a ref holds just the latest
# release; pruning drops the objects of the one it replaced.
flatpak build-update-repo "${sign[@]}" --title=lightmime --comment='lightmime releases' --homepage="$base_url/" \
    --default-branch=stable --prune "$repo"
cat >"$site/flatpak/lightmime.flatpakrepo" <<EOF
[Flatpak Repo]
Title=lightmime
Comment=lightmime releases
Url=$base_url/flatpak/repo/
Homepage=$base_url/
$gpg_key_line
EOF
echo "$app_id: $(ostree --repo="$repo" refs | tr '\n' ' ')"
