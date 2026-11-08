#!/usr/bin/env bash
# Upload the Android APK from release/ (android.yml's Collect step) to the R2 bucket served at
# https://dl.oah.dev:
#
#   android/v<version>/oah-android-<version>.apk   immutable (+ .sha256)
#   android/latest.json                            the newest APK   no-cache, uploaded last
#
# oah.dev's download link reads android/latest.json. A version is published once (versioned
# files are cached forever): if it's already there this exits without uploading.
# OAH_R2_OVERWRITE=1 overrides that (then purge the cache).
#
# Needs CLOUDFLARE_API_TOKEN (R2 edit) and CLOUDFLARE_ACCOUNT_ID, or a local `wrangler login`.
# OAH_R2_BUCKET overrides the bucket (default: oah-releases).
set -euo pipefail
cd "$(dirname "$0")/.."

BUCKET="${OAH_R2_BUCKET:-oah-releases}"
WRANGLER=(npx --yes wrangler@4.151.3)
PUBLIC="${OAH_PUBLIC_URL:-https://dl.oah.dev}"
version="$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' app/package.json | head -1)"
apk="release/oah-android-$version.apk"
[ -f "$apk" ] || { echo "$apk is missing" >&2; exit 1; }

put() { # put <bucket key> <file> <content-type> <cache-control>
  echo "  r2://$BUCKET/$1"
  "${WRANGLER[@]}" r2 object put "$BUCKET/$1" --file "$2" --content-type "$3" --cache-control "$4" --remote >/dev/null
}

if [ "${OAH_R2_OVERWRITE:-0}" != 1 ] && curl -fsI --max-time 20 "$PUBLIC/android/v$version/oah-android-$version.apk" >/dev/null 2>&1; then
  echo "oah android $version is already published at $PUBLIC — not overwriting it (cached forever)."
  exit 0
fi

sha="$(cut -d' ' -f1 "$apk.sha256")"
size="$(stat -c %s "$apk")"
path="android/v$version/oah-android-$version.apk"
cat > release/android-latest.json <<JSON
{
  "version": "$version",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "files": {
    "android-arm": { "path": "$path", "url": "$PUBLIC/$path", "sha256": "$sha", "size": $size }
  }
}
JSON

immutable="public, max-age=31536000, immutable"
echo "publishing oah android $version to $BUCKET"
put "$path" "$apk" "application/vnd.android.package-archive" "$immutable"
put "$path.sha256" "$apk.sha256" "text/plain; charset=utf-8" "$immutable"
# Last, so the download link never points at a file that isn't up yet.
put "android/latest.json" release/android-latest.json "application/json; charset=utf-8" "no-cache"
echo "done: $PUBLIC/android/latest.json → $version"
