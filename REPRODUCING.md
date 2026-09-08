# Provenance of the prebuilt binaries

The plugin ships two prebuilt ELF executables so users don't need a Rust
toolchain:

- `bin/ttfx-bg-rs-aarch64`
- `bin/ttfx-bg-rs-x86_64`

## Build environment for tag 0.0.8

Both binaries were built natively on **Arch Linux** with Rust 1.98.0.
These builds use the development checkout locations below, not the historical
`/tmp/oab-build/src` canonical path. Byte-for-byte reproduction on an arbitrary
host or checkout path has not been verified.

| Input | Value | Pinned by |
|-------|-------|-----------|
| Distro | Arch Linux (rolling) | host |
| Rust toolchain | `rustc 1.98.0 (88d9e12ae 2026-08-18)` | `rustup default 1.98.0` |
| Wrapper source | `bin/ttfx-bg-rs/src/` (including the wordmark bitmap) | git commit |
| Wrapper lockfile | `bin/ttfx-bg-rs/Cargo.lock` | git commit |
| ttfx engine (vendored) | `ttfx-src/` git submodule | exact gitlink in this tag |
| ttfx lockfile | `ttfx-src/Cargo.lock` | submodule commit |
| Release profile | `lto=true`, `strip=true` | `bin/ttfx-bg-rs/Cargo.toml` |

The submodule URL is `https://github.com/avillagran/ttfx`, branch
`audio-background-vendor`. Always use the tag's exact submodule commit, not
the branch tip. It includes the audio hooks, live color transforms and
embedding-only final-text bands.

Build checkout locations:

- aarch64: `/home/avillagran/Work/omarchy-plugins/omarchy-ttfx-background`
- x86_64: `/home/kuyen/tbg-build`

## Rebuild from the tagged source

```sh
git clone https://github.com/avillagran/omarchy-audio-background.git
cd omarchy-audio-background
git checkout 0.0.8
git submodule update --init

# Pinned toolchain
rustup default 1.98.0

# Arch build deps: base-devel rustup gtk4 gtk4-layer-shell vte4

cd bin/ttfx-bg-rs
cargo test --release --locked
cargo build --release --locked
sha256sum target/release/ttfx-bg-rs
```

Verify the shipped files from the repository root:

```sh
sha256sum -c SHA256SUMS.txt          # checks BOTH bundled binaries
```

A successful checksum check proves only that the shipped files match their
recorded hashes. It does not independently prove source-to-binary equivalence.
The repository's `.github/workflows/check-binary-hashes.yml` checks these
recorded hashes; rebuilding and comparing is a separate verification step.

## Reproduction limits

- Checkout and Cargo cache paths can affect embedded panic-location strings.
- System-library versions and toolchain packaging can affect the binary.
- Native plugin tests pass on both architectures. On aarch64 the engine's
  release-only floating-point easing golden has a known bit-level mismatch,
  reproduced on unchanged source; the debug suite and band tests pass.
