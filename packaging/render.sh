#!/usr/bin/env bash
# Fill in the Homebrew formula and cask and the AUR package for a release, from its published
# files: `packaging/render.sh 1.0.0 out/`. Needs curl and sha256sum (or shasum).
set -euo pipefail
shopt -s inherit_errexit

version=${1:?usage: render.sh <version> <out dir>}
out=${2:?usage: render.sh <version> <out dir>}
here=$(cd "$(dirname "$0")" && pwd)
repo=https://github.com/AJMerr/lucy
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

sha() {
  local file
  file="$work/$(basename "$1")"
  curl -fsSL --retry 3 -o "$file" "$1"
  if command -v sha256sum >/dev/null; then sha256sum "$file"; else shasum -a 256 "$file"; fi | cut -d' ' -f1
}

source_sha=$(sha "$repo/archive/refs/tags/v$version.tar.gz")
dmg_sha=$(sha "$repo/releases/download/v$version/Lucy_${version}_aarch64.dmg")
deb_sha=$(sha "$repo/releases/download/v$version/Lucy_${version}_amd64.deb")
mit_sha=$(sha "https://raw.githubusercontent.com/AJMerr/lucy/v$version/LICENSE-MIT")
apache_sha=$(sha "https://raw.githubusercontent.com/AJMerr/lucy/v$version/LICENSE-APACHE")

fill() {
  sed -e "s/@VERSION@/$version/g" \
    -e "s/@SOURCE_SHA256@/$source_sha/g" \
    -e "s/@DMG_SHA256@/$dmg_sha/g" \
    -e "s/@DEB_SHA256@/$deb_sha/g" \
    -e "s/@MIT_SHA256@/$mit_sha/g" \
    -e "s/@APACHE_SHA256@/$apache_sha/g" \
    "$1" > "$2"
  if grep -q '@[A-Z_0-9]*@' "$2"; then
    echo "render.sh: unfilled placeholder in $2" >&2
    exit 1
  fi
}

mkdir -p "$out/homebrew/Formula" "$out/homebrew/Casks" "$out/aur"
fill "$here/homebrew/lucy.rb" "$out/homebrew/Formula/lucy.rb"
fill "$here/homebrew/lucy-app.rb" "$out/homebrew/Casks/lucy.rb"
fill "$here/aur/PKGBUILD" "$out/aur/PKGBUILD"
fill "$here/aur/SRCINFO" "$out/aur/.SRCINFO"
echo "Rendered v$version into $out"
