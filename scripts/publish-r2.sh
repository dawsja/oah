#!/usr/bin/env bash
# Upload a release from dist/ (made by `bun scripts/build.ts --all`) to the R2
# bucket served at https://dl.oah.dev, in the layout the installers expect:
#
#   install.sh  install.ps1                      no-cache
#   v<version>/oah-<os>-<arch>.tar.gz(.sha256)   immutable
#   latest.json  latest.txt                      no-cache, uploaded last
#
# A version is published once: versioned files are cached forever, so if
# v<version> is already on dl.oah.dev this exits without uploading (bump the
# version in package.json). OAH_R2_OVERWRITE=1 overrides that — then purge the
# Cloudflare cache for those URLs.
#
# Needs CLOUDFLARE_API_TOKEN (R2 edit) and CLOUDFLARE_ACCOUNT_ID, or a local
# `wrangler login`. OAH_R2_BUCKET overrides the bucket (default: oah-releases).
set -euo pipefail
cd "$(dirname "$0")/.."

BUCKET="${OAH_R2_BUCKET:-oah-releases}"
WRANGLER=(npx --yes wrangler@4.151.3)
version="$(tr -d ' \r\n' <dist/latest.txt)"
[ -n "$version" ] || { echo "dist/latest.txt is empty — run bun scripts/build.ts --all first" >&2; exit 1; }
if [ -n "${GITHUB_REF_NAME:-}" ] && [ "${GITHUB_REF_NAME#v}" != "$version" ]; then
  echo "tag $GITHUB_REF_NAME doesn't match package.json version $version" >&2
  exit 1
fi

put() { # put <bucket key> <file> <content-type> <cache-control>
  echo "  r2://$BUCKET/$1"
  "${WRANGLER[@]}" r2 object put "$BUCKET/$1" --file "$2" --content-type "$3" --cache-control "$4" --remote >/dev/null
}

PUBLIC="${OAH_PUBLIC_URL:-https://dl.oah.dev}"
if [ "${OAH_R2_OVERWRITE:-0}" != 1 ] && curl -fsI --max-time 20 "$PUBLIC/v$version/oah-linux-x64.tar.gz.sha256" >/dev/null 2>&1; then
  echo "oah $version is already published at $PUBLIC — not overwriting it (cached forever)."
  echo "Bump \"version\" in package.json for a new release, or set OAH_R2_OVERWRITE=1 and purge the cache."
  exit 0
fi
[ -f dist/latest.json ] || { echo "dist/latest.json is missing — run bun scripts/build.ts --all first" >&2; exit 1; }

shopt -s nullglob
archives=(dist/oah-*.tar.gz)
[ ${#archives[@]} -gt 0 ] || { echo "no archives in dist/" >&2; exit 1; }

echo "publishing oah $version to $BUCKET"
for f in "${archives[@]}"; do
  name="$(basename "$f")"
  put "v$version/$name" "$f" "application/gzip" "public, max-age=31536000, immutable"
  put "v$version/$name.sha256" "$f.sha256" "text/plain; charset=utf-8" "public, max-age=31536000, immutable"
done
put "install.sh" "scripts/install.sh" "text/x-shellscript; charset=utf-8" "no-cache"
put "install.ps1" "scripts/install.ps1" "text/plain; charset=utf-8" "no-cache"
# Last, so installers and `oah update` never see a version whose files aren't up yet.
put "latest.json" "dist/latest.json" "application/json; charset=utf-8" "no-cache"
put "latest.txt" "dist/latest.txt" "text/plain; charset=utf-8" "no-cache"
echo "done: https://dl.oah.dev/latest.txt → $version"
