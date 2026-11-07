#!/bin/sh
# oah desktop installer (Linux)
#
#   curl -fsSL https://dl.oah.dev/install-desktop.sh | sh
#
# Installs the .deb with apt, the .rpm with dnf/zypper, else the AppImage in
# ~/.local/bin with a menu entry. Installed apps update themselves.
#
# Options (environment variables):
#   OAH_DESKTOP_VERSION  version to install, e.g. 0.1.4 (default: latest)
#   OAH_DESKTOP_FORMAT   deb | rpm | appimage (default: what this system uses)
#   OAH_DOWNLOAD_URL     where release files live (default: https://dl.oah.dev)
#                        layout: $OAH_DOWNLOAD_URL/desktop/latest.txt
#                                $OAH_DOWNLOAD_URL/desktop/v<version>/oah-desktop-<version>-<os>-<arch>.<ext>[.sha256]
set -eu

if [ -t 1 ]; then
  B="$(printf '\033[1m')"; D="$(printf '\033[2m')"; V="$(printf '\033[38;5;220m')"; G="$(printf '\033[32m')"; R="$(printf '\033[31m')"; N="$(printf '\033[0m')"
else
  B=""; D=""; V=""; G=""; R=""; N=""
fi
say() { printf '%s\n' "$*"; }
fail() { printf '%serror%s: %s\n' "$R" "$N" "$*" >&2; exit 1; }

BASE="${OAH_DOWNLOAD_URL:-https://dl.oah.dev}"
BASE="${BASE%/}"

fetch() { # fetch URL [OUTFILE]
  if command -v curl >/dev/null 2>&1; then
    if [ -n "${2:-}" ]; then curl -fSL --retry 3 --progress-bar -o "$2" "$1"; else curl -fsSL --retry 3 "$1"; fi
  elif command -v wget >/dev/null 2>&1; then
    if [ -n "${2:-}" ]; then wget -q --show-progress -O "$2" "$1"; else wget -q -O - "$1"; fi
  else
    fail "curl or wget is required"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# --- platform ----------------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux) os=linux ;;
  Darwin) fail "oah is for Linux and Windows for now" ;;
  MINGW* | MSYS* | CYGWIN*) fail "on Windows, run this in PowerShell instead:  irm https://dl.oah.dev/install-desktop.ps1 | iex" ;;
  *) fail "unsupported OS: $os" ;;
esac
case "$arch" in
  x86_64 | amd64) arch=x64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) fail "unsupported architecture: $arch" ;;
esac
if [ "$arch" != x64 ]; then
  fail "oah is built for x64 Linux only for now"
fi

# --- what to download ------------------------------------------------------------
ext="${OAH_DESKTOP_FORMAT:-}"
if [ -z "$ext" ]; then
  if command -v apt-get >/dev/null 2>&1 && command -v dpkg >/dev/null 2>&1; then ext=deb
  elif command -v dnf >/dev/null 2>&1 || command -v zypper >/dev/null 2>&1; then ext=rpm
  else ext=appimage; fi
fi
case "$ext" in deb | rpm) ;; appimage) ext=AppImage ;; AppImage) ;; *) fail "OAH_DESKTOP_FORMAT must be deb, rpm or appimage" ;; esac

version="${OAH_DESKTOP_VERSION:-latest}"
if [ "$version" = latest ]; then
  version="$(fetch "$BASE/desktop/latest.txt" 2>/dev/null | tr -d ' \r\n' || true)"
  [ -n "$version" ] || fail "couldn't determine the latest version from $BASE/desktop/latest.txt"
fi
version="${version#v}"

say ""
say "  ${V}${B}◆ oah${N} ${D}desktop installer${N}"
say "  ${D}version${N} $version  ${D}platform${N} $os-$arch ($ext)"
say ""

# --- download & verify ------------------------------------------------------------
tmp="$(mktemp -d 2>/dev/null || mktemp -d -t oah)"
trap 'rm -rf "$tmp"' EXIT INT TERM
file="oah-desktop-$version-$os-$arch.$ext"
url="$BASE/desktop/v$version/$file"
say "  ${D}downloading${N} $url"
fetch "$url" "$tmp/$file" || fail "download failed: $url"
want="$(fetch "$url.sha256" 2>/dev/null | cut -d' ' -f1 | tr -d '\r\n' || true)"
[ -n "$want" ] || fail "couldn't fetch the checksum for $file"
[ "$want" = "$(sha256 "$tmp/$file")" ] || fail "checksum mismatch for $file"
say "  ${D}checksum${N}    ${G}ok${N}"

# --- install ---------------------------------------------------------------------
sudo_() { if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi; }
case "$ext" in
  deb)
    say "  ${D}installing with apt (may ask for your password)${N}"
    sudo_ apt-get install -y "$tmp/$file" || fail "apt couldn't install $file"
    run="oah-desktop"
    ;;
  rpm)
    say "  ${D}installing the rpm (may ask for your password)${N}"
    if command -v dnf >/dev/null 2>&1; then sudo_ dnf install -y "$tmp/$file"; else sudo_ zypper --non-interactive install --allow-unsigned-rpm "$tmp/$file"; fi || fail "couldn't install $file"
    run="oah-desktop"
    ;;
  AppImage)
    bin="$HOME/.local/bin"
    mkdir -p "$bin" "$HOME/.local/share/applications"
    mv -f "$tmp/$file" "$bin/oah-desktop.AppImage"
    chmod 755 "$bin/oah-desktop.AppImage"
    cat >"$HOME/.local/share/applications/oah-desktop.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=oah
Comment=open agent harness
Exec=$bin/oah-desktop.AppImage
Icon=utilities-terminal
Categories=Development;
Terminal=false
EOF
    say "  ${D}installed${N}   $bin/oah-desktop.AppImage"
    command -v fusermount >/dev/null 2>&1 || say "  ${D}note: AppImages need FUSE 2 (e.g. sudo apt install libfuse2)${N}"
    run="$bin/oah-desktop.AppImage"
    ;;
esac

say ""
say "  ${G}${B}oah desktop $version is installed.${N} Open it from your apps, or run: ${B}$run${N}"
say "  ${D}It updates itself.${N}"
say ""
