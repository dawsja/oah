#!/bin/sh
# oah installer
#
#   curl -fsSL https://oah.dev/install.sh | sh
#
# Options (environment variables):
#   OAH_VERSION        version to install, e.g. 0.1.0 (default: latest)
#   OAH_INSTALL        install root (default: ~/.config/oah); the binary goes in $OAH_INSTALL/bin
#   OAH_DOWNLOAD_URL   where release files live (default: https://oah.dev/download)
#                      layout: $OAH_DOWNLOAD_URL/latest.txt
#                              $OAH_DOWNLOAD_URL/v<version>/oah-<os>-<arch>.tar.gz[.sha256]
#                      GitHub releases use the same layout:
#                      OAH_DOWNLOAD_URL=https://github.com/dawsja/oah/releases/download
#   OAH_NO_MODIFY_PATH set to 1 to leave shell profiles alone
set -eu

BASE="${OAH_DOWNLOAD_URL:-https://oah.dev/download}"
BASE="${BASE%/}"
ROOT="${OAH_INSTALL:-${XDG_CONFIG_HOME:-$HOME/.config}/oah}"
BIN_DIR="$ROOT/bin"
GITHUB_LATEST="https://github.com/dawsja/oah/releases/latest/download/latest.txt"

if [ -t 1 ]; then
  B="$(printf '\033[1m')"; D="$(printf '\033[2m')"; V="$(printf '\033[38;5;141m')"; G="$(printf '\033[32m')"; R="$(printf '\033[31m')"; N="$(printf '\033[0m')"
else
  B=""; D=""; V=""; G=""; R=""; N=""
fi
say() { printf '%s\n' "$*"; }
fail() { printf '%serror%s: %s\n' "$R" "$N" "$*" >&2; exit 1; }

fetch() { # fetch URL [OUTFILE]
  if command -v curl >/dev/null 2>&1; then
    if [ -n "${2:-}" ]; then curl -fsSL --retry 3 -o "$2" "$1"; else curl -fsSL --retry 3 "$1"; fi
  elif command -v wget >/dev/null 2>&1; then
    if [ -n "${2:-}" ]; then wget -q -O "$2" "$1"; else wget -q -O - "$1"; fi
  else
    fail "curl or wget is required"
  fi
}

# --- platform ----------------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux) os=linux ;;
  Darwin) os=darwin ;;
  MINGW* | MSYS* | CYGWIN*) fail "on Windows, run this in PowerShell instead:  irm https://oah.dev/install.ps1 | iex" ;;
  *) fail "unsupported OS: $os" ;;
esac
case "$arch" in
  x86_64 | amd64) arch=x64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) fail "unsupported architecture: $arch" ;;
esac
# An x64 shell under Rosetta on Apple Silicon should get the native build.
if [ "$os" = darwin ] && [ "$arch" = x64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ]; then
  arch=arm64
fi
if [ "$os" = linux ] && [ -f /etc/alpine-release ]; then
  fail "musl-based Linux (Alpine) isn't supported yet."
fi
target="$os-$arch"

# --- version -------------------------------------------------------------------
version="${OAH_VERSION:-latest}"
if [ "$version" = latest ]; then
  version="$(fetch "$BASE/latest.txt" 2>/dev/null || fetch "$GITHUB_LATEST" 2>/dev/null || true)"
  version="$(printf '%s' "$version" | tr -d ' \r\n')"
  [ -n "$version" ] || fail "couldn't determine the latest version from $BASE/latest.txt"
fi
version="${version#v}"

say ""
say "  ${V}${B}◆ oah${N} ${D}installer${N}"
say "  ${D}version${N} $version  ${D}platform${N} $target"
say ""

# --- download & verify ------------------------------------------------------------
tmp="$(mktemp -d 2>/dev/null || mktemp -d -t oah)"
trap 'rm -rf "$tmp"' EXIT INT TERM
file="oah-$target.tar.gz"
url="$BASE/v$version/$file"
say "  ${D}downloading${N} $url"
fetch "$url" "$tmp/$file" || fail "download failed: $url"

if fetch "$url.sha256" "$tmp/$file.sha256" 2>/dev/null; then
  want="$(cut -d' ' -f1 <"$tmp/$file.sha256" | tr -d '\r\n')"
  if command -v sha256sum >/dev/null 2>&1; then
    got="$(sha256sum "$tmp/$file" | cut -d' ' -f1)"
  else
    got="$(shasum -a 256 "$tmp/$file" | cut -d' ' -f1)"
  fi
  [ "$want" = "$got" ] || fail "checksum mismatch for $file (expected $want, got $got)"
  say "  ${D}checksum${N}    ${G}ok${N}"
fi

# --- install ---------------------------------------------------------------------
mkdir -p "$BIN_DIR"
tar -xzf "$tmp/$file" -C "$tmp"
[ -f "$tmp/oah" ] || fail "archive doesn't contain the oah binary"
mv -f "$tmp/oah" "$BIN_DIR/oah"
chmod 755 "$BIN_DIR/oah"
if [ "$os" = darwin ]; then
  xattr -d com.apple.quarantine "$BIN_DIR/oah" 2>/dev/null || true
fi
say "  ${D}installed${N}   $BIN_DIR/oah"

# --- PATH ------------------------------------------------------------------------
on_path=0
case ":$PATH:" in *":$BIN_DIR:"*) on_path=1 ;; esac

if [ "$on_path" = 0 ] && [ "${OAH_NO_MODIFY_PATH:-0}" != 1 ]; then
  line="export PATH=\"$BIN_DIR:\$PATH\""
  shell_name="$(basename "${SHELL:-sh}")"
  case "$shell_name" in
    zsh) profile="${ZDOTDIR:-$HOME}/.zshrc" ;;
    bash)
      if [ "$os" = darwin ]; then profile="$HOME/.bash_profile"; else profile="$HOME/.bashrc"; fi
      ;;
    fish)
      profile="${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish"
      line="fish_add_path \"$BIN_DIR\""
      ;;
    *) profile="$HOME/.profile" ;;
  esac
  mkdir -p "$(dirname "$profile")"
  if ! grep -qs "$BIN_DIR" "$profile"; then
    printf '\n# oah\n%s\n' "$line" >>"$profile"
    say "  ${D}PATH${N}        added to $profile"
  fi
fi

say ""
say "  ${G}✓${N} ${B}oah $version is installed.${N}"
if [ "$on_path" = 0 ]; then
  say "  Open a new terminal (or run ${B}export PATH=\"$BIN_DIR:\$PATH\"${N}), then:"
else
  say "  Run:"
fi
say ""
say "    ${B}oah${N}           ${D}start in the current directory${N}"
say "    ${B}oah login${N}     ${D}continue with ChatGPT — or /auth inside oah for Anthropic / OpenAI keys${N}"
say ""
