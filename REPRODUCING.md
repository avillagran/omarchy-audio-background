# Provenance of the prebuilt binaries

The plugin ships two prebuilt ELF executables so users don't need a Rust
toolchain:

- `bin/ttfx-bg-rs-aarch64`
- `bin/ttfx-bg-rs-x86_64`

## Canonical build environment

The binaries are built on **Arch Linux** hosts (the same distro family the
plugin targets), at a **canonical checkout path** so no machine-specific
paths are embedded in the binaries, with the pinned toolchain:

| Input | Value | Pinned by |
|-------|-------|-----------|
| Distro | Arch Linux (rolling, gtk4 4.22.x, vte4 0.84.x, gtk4-layer-shell 1.3.0) | host |
| Rust toolchain | `rustc 1.98.0 (88d9e12ae 2026-08-18)` | `rustup default 1.98.0` |
| Wrapper source | `bin/ttfx-bg-rs/src/main.rs` | git commit |
| Wrapper lockfile | `bin/ttfx-bg-rs/Cargo.lock` | git commit |
| ttfx engine (vendored) | `ttfx-src/` git submodule @ `7e4a9b2` | submodule pointer + `.gitmodules` |
| ttfx lockfile | `ttfx-src/Cargo.lock` | submodule commit |
| Release profile | `lto=true`, `strip=true`, `codegen-units=1` | `bin/ttfx-bg-rs/Cargo.toml` |

The submodule URL is `https://github.com/avillagran/ttfx` branch
`audio-background-vendor`, which carries the two engine commits this
plugin builds on top of upstream ttfx 0.3.2:

- `d6c4046` — audio-reactive color: global sgr_color hook
- `7e4a9b2` — audio-reactive thunderstorm (on_audio reference impl)

## Reproduce byte-for-byte

```sh
# Canonical path - this exact directory matters: it is what gets baked
# into the binary instead of the builder's home directory.
CANON=/tmp/oab-build
rm -rf "$CANON" && mkdir -p "$CANON"

git clone https://github.com/avillagran/omarchy-audio-background.git "$CANON/src"
cd "$CANON/src"
git checkout <commit-you-are-verifying>
git submodule update --init

# Pinned toolchain
rustup default 1.98.0

# Arch build deps: base-devel rustup gtk4 gtk4-layer-shell vte4

cd bin/ttfx-bg-rs
cargo build --release
sha256sum target/release/ttfx-bg-rs
```

Compare the result against `SHA256SUMS.txt` in the repo root and against
the bundled binary for your architecture:

```sh
sha256sum -c SHA256SUMS.txt          # checks BOTH bundled binaries
```

A green `sha256sum -c` means the bytes a user installs are exactly what
the reviewed source and lockfiles produce. The repository's CI runs the
same check on every change that touches the binaries
(`.github/workflows/check-binary-hashes.yml`), so a binary can never
drift from its recorded hash undetected.

## Determinism evidence

- Two consecutive `cargo build --release` runs from the same tree at the
  canonical path produce identical SHA-256 (verified repeatedly during
  development, including after `cargo clean`).
- Builds from different checkout paths do NOT hash identically: rustc
  embeds source paths in panic-location strings. The canonical path
  `/tmp/oab-build/src` is part of the recipe for that reason.
- Cross-distro rebuilds (e.g. Ubuntu) may also differ in system-library
  linkage details; use Arch for an exact match.
