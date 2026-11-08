#!/usr/bin/env bash
# plaindoc installer: downloads the release binary for this machine, checks its sha256 and puts
# it into ~/.local/bin.
#
#   curl -fsSL https://raw.githubusercontent.com/fixcik/plaindoc/main/install.sh | bash
#
# Environment:
#   PLAINDOC_VERSION      version to install, e.g. 0.1.0 or v0.1.0 (default: the latest release)
#   PLAINDOC_INSTALL_DIR  where to put the binary (default: $HOME/.local/bin)
#   PLAINDOC_BASE_URL     releases base URL (default: https://github.com/fixcik/plaindoc/releases);
#                         <base>/download/v<version>/<file> must serve the archive and SHA256SUMS
set -euo pipefail

REPO="fixcik/plaindoc"
BASE_URL="${PLAINDOC_BASE_URL:-https://github.com/$REPO/releases}"
INSTALL_DIR="${PLAINDOC_INSTALL_DIR:-$HOME/.local/bin}"

# The prebuilt static ONNX Runtime inside the binary needs this glibc (see docs/building.md).
MIN_GLIBC="2.38"

die() {
  echo "plaindoc install: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "'$1' is required but not found"
}

# 0 when $1 >= $2 (dotted versions).
version_ge() {
  [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -n 1)" = "$2" ]
}

detect_target() {
  local os arch
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Darwin)
      case "$arch" in
        arm64) echo "aarch64-apple-darwin" ;;
        *) die "no prebuilt binary for macOS $arch (only Apple silicon); build from source: https://github.com/$REPO#build-and-run" ;;
      esac ;;
    Linux)
      local libc
      libc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}' || true)
      [ -n "$libc" ] || die "glibc not found (musl, e.g. Alpine, is not supported); build from source: https://github.com/$REPO#build-and-run"
      version_ge "$libc" "$MIN_GLIBC" \
        || die "glibc $libc is older than the required $MIN_GLIBC (Ubuntu 24.04, Debian 13, Fedora 39, RHEL 10 or newer); see https://github.com/$REPO/blob/main/docs/building.md#linux-requirements"
      case "$arch" in
        x86_64 | amd64) echo "x86_64-unknown-linux-gnu" ;;
        aarch64 | arm64) echo "aarch64-unknown-linux-gnu" ;;
        *) die "no prebuilt binary for Linux $arch" ;;
      esac ;;
    MINGW* | MSYS* | CYGWIN*)
      die "on Windows download the .zip from https://github.com/$REPO/releases and put plaindoc.exe on PATH" ;;
    *) die "unsupported system: $os" ;;
  esac
}

latest_version() {
  local url
  url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$BASE_URL/latest") \
    || die "cannot find the latest release at $BASE_URL/latest"
  case "$url" in
    */tag/v*) echo "${url##*/tag/v}" ;;
    *) die "no releases yet at $BASE_URL (set PLAINDOC_VERSION to install a specific one)" ;;
  esac
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "sha256sum or shasum is required to verify the download"
  fi
}

main() {
  need curl
  need tar
  need uname

  local target version name archive dl want got
  target=$(detect_target)
  version="${PLAINDOC_VERSION:-}"
  [ -n "$version" ] || version=$(latest_version)
  version="${version#v}"

  name="plaindoc-$version-$target"
  archive="$name.tar.gz"
  dl="$BASE_URL/download/v$version"

  # Global: the EXIT trap runs after main has returned.
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT

  echo "Installing plaindoc $version ($target) into $INSTALL_DIR"
  curl -fsSL "$dl/$archive" -o "$tmp/$archive" || die "download failed: $dl/$archive"
  curl -fsSL "$dl/SHA256SUMS" -o "$tmp/SHA256SUMS" || die "download failed: $dl/SHA256SUMS"

  want=$(awk -v f="$archive" '$2 == f || $2 == "*" f {print $1}' "$tmp/SHA256SUMS")
  [ -n "$want" ] || die "$archive is not listed in SHA256SUMS"
  got=$(sha256_of "$tmp/$archive")
  [ "$want" = "$got" ] || die "sha256 mismatch for $archive (expected $want, got $got)"

  tar -xzf "$tmp/$archive" -C "$tmp"
  [ -f "$tmp/$name/plaindoc" ] || die "the archive has no plaindoc binary"

  mkdir -p "$INSTALL_DIR"
  install -m 755 "$tmp/$name/plaindoc" "$INSTALL_DIR/plaindoc.new"
  mv -f "$INSTALL_DIR/plaindoc.new" "$INSTALL_DIR/plaindoc"

  "$INSTALL_DIR/plaindoc" --version
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Add $INSTALL_DIR to PATH, e.g.: export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
  esac
}

# Everything runs from one call, so a download cut off in the middle cannot execute half a script.
main "$@"
