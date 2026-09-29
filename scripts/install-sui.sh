#!/usr/bin/env bash
# Installs the pinned `sui` CLI into .tools/ (git-ignored). Used only for fixtures and
# oracle tests (`sui move build`, `sui move summary`); generation itself never needs it.
set -euo pipefail
VERSION="${SUI_VERSION:-mainnet-v1.80.1}"
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/.tools/sui-$VERSION"
if [ -x "$dest/sui" ]; then echo "$dest/sui"; exit 0; fi
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) asset="sui-$VERSION-ubuntu-x86_64.tgz" ;;
  Linux-aarch64) asset="sui-$VERSION-ubuntu-aarch64.tgz" ;;
  Darwin-arm64) asset="sui-$VERSION-macos-arm64.tgz" ;;
  *) echo "unsupported platform" >&2; exit 1 ;;
esac
mkdir -p "$dest"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
gh release download "$VERSION" -R MystenLabs/sui -p "$asset" -D "$tmp"
tar -xzf "$tmp/$asset" -C "$dest" --wildcards '*sui' --no-anchored --transform 's#.*/##' 2>/dev/null \
  || tar -xzf "$tmp/$asset" -C "$dest" sui
chmod +x "$dest/sui"
ln -sfn "$dest" "$root/.tools/sui"
echo "$dest/sui"
