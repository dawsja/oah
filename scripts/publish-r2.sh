#!/usr/bin/env bash
# Upload a release from dist/ (made by `bun scripts/build.ts --all`) to the R2
# bucket served at https://dl.oah.dev, in the layout the installers expect:
#
#   install.sh  install.ps1                      no-cache
#   v<version>/oah-<os>-<arch>.tar.gz(.sha256)   immutable
#   latest.txt                                   no-cache, uploaded last
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
# Last, so installers never see a version whose files aren't up yet.
put "latest.txt" "dist/latest.txt" "text/plain; charset=utf-8" "no-cache"
echo "done: https://dl.oah.dev/latest.txt → $version"
