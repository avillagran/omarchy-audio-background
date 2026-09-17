#!/bin/sh
# Reproducible release build for the prebuilt ttfx-bg-rs binaries.
#
# Canonicalizes every path the compiler can embed (crate + ttfx engine +
# target dir) to /oab/build so the output depends only on source, lockfiles,
# toolchain and system libraries -- never on the checkout location.
#
# Usage: scripts/repro-build.sh          (from anywhere in the repo)
# Requires: rustup toolchain 1.98.0 (see REPRODUCING.md)
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
crate="$repo_root/bin/ttfx-bg-rs"
engine="$repo_root/ttfx-src"

cd "$crate"
rm -rf target
RUSTFLAGS="--remap-path-prefix=$crate=/oab/build/bin/ttfx-bg-rs \
--remap-path-prefix=$engine=/oab/build/ttfx-src \
--remap-path-prefix=$crate/target=/oab/build/target" \
  cargo build --release --locked

echo "built: $crate/target/release/ttfx-bg-rs"
sha256sum "$crate/target/release/ttfx-bg-rs"
