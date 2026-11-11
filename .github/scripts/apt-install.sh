#!/usr/bin/env bash
# Installs Ubuntu packages on a GitHub runner, outliving an outage of the runner's own package
# mirror (azure.archive.ubuntu.com stopped resolving on 2026-11-11 and failed every Linux job):
# a try as the runner is set up, then from Ubuntu's own servers.
#
#   .github/scripts/apt-install.sh <package>...
set -uo pipefail

apt() {
  # A mirror that doesn't answer gives up in seconds, not after apt's own long waits.
  sudo timeout 240 apt-get -o Acquire::Retries=2 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 "$@"
}

try() {
  apt update -q && apt install -y -q "$@"
}

for round in 1 2 3; do
  if try "$@"; then exit 0; fi
  echo "::warning::apt failed (round $round); trying Ubuntu's own servers"
  # Off the runner's mirror list (mirror+file:/etc/apt/apt-mirrors.txt): Ubuntu's main archive
  # and security servers.
  codename=$(. /etc/os-release && echo "$VERSION_CODENAME")
  printf 'deb http://archive.ubuntu.com/ubuntu %s main universe restricted multiverse\ndeb http://archive.ubuntu.com/ubuntu %s-updates main universe restricted multiverse\ndeb http://security.ubuntu.com/ubuntu %s-security main universe restricted multiverse\n' \
    "$codename" "$codename" "$codename" | sudo tee /etc/apt/sources.list >/dev/null
  sudo rm -f /etc/apt/sources.list.d/ubuntu.sources
  sleep $((round * 10))
done
echo "::error::couldn't install $*"
exit 1
