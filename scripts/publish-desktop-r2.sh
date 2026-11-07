#!/usr/bin/env bash
# Upload a desktop release from app/release/ (scripts/collect.ts per target,
# then scripts/manifest.ts) to the R2 bucket served at https://dl.oah.dev:
#
#   desktop/v<version>/oah-desktop-<version>-<platform>.<ext>  immutable
#                                       (+ .sha256, and .sig for the updater)
#   desktop/v<version>/latest.json      this release's manifest  immutable
#   desktop/latest.json                 the newest release       no-cache, uploaded last
#
# The app's background updater reads desktop/latest.json; so do oah.dev's download links.
# A version is published once (versioned files are cached forever): if it's
# already on dl.oah.dev this exits without uploading — bump "version" in
# app/package.json. OAH_R2_OVERWRITE=1 overrides that (then purge the cache).
#
# Needs CLOUDFLARE_API_TOKEN (R2 edit) and CLOUDFLARE_ACCOUNT_ID, or a local
# `wrangler login`. OAH_R2_BUCKET overrides the bucket (default: oah-releases).
set -euo pipefail
cd "$(dirname "$0")/.."

BUCKET="${OAH_R2_BUCKET:-oah-releases}"
WRANGLER=(npx --yes wrangler@4.151.3)
REL=app/release
version="$(tr -d ' \r\n' <"$REL/latest.txt" 2>/dev/null || true)"
[ -n "$version" ] || { echo "$REL/latest.txt is missing — run app/scripts/collect.ts and manifest.ts first" >&2; exit 1; }
want="$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' app/package.json | head -1)"
[ "$version" = "$want" ] || { echo "release is $version but app/package.json says $want" >&2; exit 1; }
if [ -n "${GITHUB_REF_NAME:-}" ] && [[ "$GITHUB_REF_NAME" == v* ]] && [ "${GITHUB_REF_NAME#v}" != "$version" ]; then
  echo "tag $GITHUB_REF_NAME doesn't match app/package.json version $version" >&2
  exit 1
fi

put() { # put <bucket key> <file> <content-type> <cache-control>
  echo "  r2://$BUCKET/$1"
  "${WRANGLER[@]}" r2 object put "$BUCKET/$1" --file "$2" --content-type "$3" --cache-control "$4" --remote >/dev/null
}

PUBLIC="${OAH_PUBLIC_URL:-https://dl.oah.dev}"
if [ "${OAH_R2_OVERWRITE:-0}" != 1 ] && curl -fsI --max-time 20 "$PUBLIC/desktop/v$version/latest.json" >/dev/null 2>&1; then
  echo "oah desktop $version is already published at $PUBLIC — not overwriting it (cached forever)."
  echo "Bump \"version\" in app/package.json for a new release, or set OAH_R2_OVERWRITE=1 and purge the cache."
  exit 0
fi

shopt -s nullglob
files=("$REL"/oah-desktop-"$version"-*)
[ ${#files[@]} -gt 0 ] || { echo "no oah-desktop-$version-* files in $REL" >&2; exit 1; }

immutable="public, max-age=31536000, immutable"
echo "publishing oah desktop $version to $BUCKET"
for f in "${files[@]}"; do
  name="$(basename "$f")"
  case "$name" in
    *.sha256 | *.sig) type="text/plain; charset=utf-8" ;;
    *.deb) type="application/vnd.debian.binary-package" ;;
    *.rpm) type="application/x-rpm" ;;
    *.msi) type="application/x-msi" ;;
    *.exe) type="application/vnd.microsoft.portable-executable" ;;
    *.tar.gz) type="application/gzip" ;;
    *) type="application/octet-stream" ;;
  esac
  put "desktop/v$version/$name" "$f" "$type" "$immutable"
done
put "desktop/v$version/latest.json" "$REL/latest.json" "application/json; charset=utf-8" "$immutable"
# Last, so the updater and download links never see a version whose files aren't up yet.
put "desktop/latest.json" "$REL/latest.json" "application/json; charset=utf-8" "no-cache"
echo "done: $PUBLIC/desktop/latest.json → $version"
