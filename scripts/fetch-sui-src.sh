#!/usr/bin/env bash
# Sparse checkout of Mysten's Move crates at the pinned Sui tag into .tools/sui-src (git-ignored).
# Only the oracle tests use it: Mysten's implementation is a test oracle, never a dependency.
set -euo pipefail
TAG="${SUI_TAG:-mainnet-v1.80.1}"
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/.tools/sui-src"
if [ -d "$dest/.git" ] && [ "$(git -C "$dest" describe --tags --exact-match 2>/dev/null)" = "$TAG" ]; then
  echo "$dest"; exit 0
fi
rm -rf "$dest"
git clone -q --depth 1 --branch "$TAG" --filter=blob:none --sparse https://github.com/MystenLabs/sui.git "$dest"
git -C "$dest" sparse-checkout set --no-cone \
  /LICENSE \
  /external-crates/move/Cargo.toml /external-crates/move/Cargo.lock \
  /external-crates/move/crates/move-core-types/ \
  /external-crates/move/crates/move-binary-format/ \
  /external-crates/move/crates/move-proc-macros/ \
  /external-crates/move/crates/enum-compat-util/ \
  /external-crates/move/crates/move-abstract-interpreter/ \
  /external-crates/move/crates/move-command-line-common/
echo "$dest"
