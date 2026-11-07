#!/bin/sh
# oah has no rounded corners: fails on any Tailwind rounded-* class other than rounded-none,
# and on any border-radius in CSS or inline styles that isn't 0.
cd "$(dirname "$0")/../app/src" || exit 1
bad=$(grep -rnE '(^|[^a-z-])rounded(-(xs|sm|md|lg|xl|2xl|3xl|4xl|full|t|b|l|r|s|e|tl|tr|bl|br|ss|se|es|ee)[a-z0-9-]*|-\[[^]]*\])?([^a-z0-9-]|$)' --include='*.ts' --include='*.tsx' . | grep -vE 'rounded-none')
css=$(grep -rnE 'border-radius:[[:space:]]*[^0[:space:];]' --include='*.css' . ; grep -rnE 'borderRadius:[[:space:]]*[^0[:space:]]' --include='*.tsx' .)
if [ -n "$bad$css" ]; then
  echo "Rounded corners found (oah is square everywhere):"
  [ -n "$bad" ] && echo "$bad"
  [ -n "$css" ] && echo "$css"
  exit 1
fi
echo "check-sharp: ok"
