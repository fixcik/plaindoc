#!/usr/bin/env bash
# Packs a release binary with the license files into dist/:
#   plaindoc-<version>-<target>.tar.gz (Windows targets: .zip)
# and prints its sha256. The version comes from [workspace.package] in Cargo.toml.
# Usage: scripts/package-release.sh TARGET [BINARY] [OUT_DIR]
#   BINARY defaults to target/TARGET/release/plaindoc[.exe], OUT_DIR to dist (both in the repo).
# Works on macOS, Linux and Git Bash on Windows (zip via 7z or PowerShell).
set -euo pipefail

target="${1:?usage: $0 TARGET [BINARY] [OUT_DIR]}"
root=$(cd "$(dirname "$0")/.." && pwd)
exe=""
case "$target" in *windows*) exe=".exe" ;; esac
bin="${2:-$root/target/$target/release/plaindoc$exe}"
out="${3:-$root/dist}"

version=$(awk '
  /^\[/ { in_pkg = ($0 == "[workspace.package]") }
  in_pkg && /^version *=/ { gsub(/.*= *"|".*/, ""); print; exit }
' "$root/Cargo.toml")
[ -n "$version" ] || { echo "no version in [workspace.package] of Cargo.toml" >&2; exit 1; }
[ -f "$bin" ] || { echo "binary not found: $bin" >&2; exit 1; }

name="plaindoc-$version-$target"
mkdir -p "$out"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/$name"
cp "$bin" "$stage/$name/plaindoc$exe"
for f in LICENSE-MIT LICENSE-APACHE NOTICE README.md; do cp "$root/$f" "$stage/$name/"; done

if [ -n "$exe" ]; then
  archive="$name.zip"
  rm -f "$out/$archive"
  if command -v 7z >/dev/null; then
    (cd "$stage" && 7z a -tzip -bd "$archive" "$name" >/dev/null)
  else
    (cd "$stage" && powershell -NoProfile -Command "Compress-Archive -Path '$name' -DestinationPath '$archive'")
  fi
else
  archive="$name.tar.gz"
  tar -C "$stage" -czf "$stage/$archive" "$name"
fi
mv "$stage/$archive" "$out/$archive"

if command -v sha256sum >/dev/null; then
  (cd "$out" && sha256sum "$archive")
else
  (cd "$out" && shasum -a 256 "$archive")
fi
