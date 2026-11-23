#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
version=$(cat VERSION)
archive="ghostty-vt-${version}.xcframework.zip"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl --fail --location --silent --show-error "https://release.files.ghostty.org/libghostty-vt/${version}/${archive}" -o "$work/$archive"
expected=$(awk -v archive="$archive" '$2 == archive {print $1}' SHA256SUMS)
[[ -n "$expected" ]]
actual=$(shasum -a 256 "$work/$archive" | awk '{print $1}')
[[ "$actual" == "$expected" ]] || { echo 'Ghostty archive checksum mismatch' >&2; exit 1; }
unzip -q "$work/$archive" -d "$work"
platform="$work/ghostty-vt.xcframework/macos-arm64_x86_64"
expected=$(awk '$2 == "libghostty-vt.a" {print $1}' SHA256SUMS)
actual=$(shasum -a 256 "$platform/libghostty-vt.a" | awk '{print $1}')
[[ "$actual" == "$expected" ]] || { echo 'Ghostty static library checksum mismatch' >&2; exit 1; }
cp "$platform/libghostty-vt.a" libghostty-vt.a
cp -R "$platform/Headers/." include/
