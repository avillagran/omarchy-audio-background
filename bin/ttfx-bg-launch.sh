#!/bin/sh
# Launch the Rust audio-reactive background for the current architecture.
# The repo ships one prebuilt binary per supported arch; this script picks the
# right one so the plugin "just works" after `omarchy plugin add` with no build
# step. Add a new arch by dropping bin/ttfx-bg-rs-<uname -m> and a case below.
set -e
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ARCH=$(uname -m)
case "$ARCH" in
  aarch64|arm64) exec "$DIR/ttfx-bg-rs-aarch64" "$@" ;;
  x86_64|amd64)  exec "$DIR/ttfx-bg-rs-x86_64" "$@" ;;
  *) echo "ttfx-bg: unsupported architecture: $ARCH" >&2; exit 1 ;;
esac
