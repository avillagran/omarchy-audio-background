# Reproducing the prebuilt binaries

The plugin ships two prebuilt ELF executables so users don't need a Rust
toolchain:

- `bin/ttfx-bg-rs-aarch64`
- `bin/ttfx-bg-rs-x86_64`

This document ties each binary to the exact source tree and lockfile in
this repository, so a reviewer can independently verify that the bundled
bytes come from the reviewed source and nothing else.

## What goes into a binary

| Input | Location | Pinned by |
|-------|----------|-----------|
| Background wrapper source | `bin/ttfx-bg-rs/src/main.rs` | git commit |
| Dependency lockfile (wrapper) | `bin/ttfx-bg-rs/Cargo.lock` | git commit |
| ttfx effects engine (vendored) | `ttfx-src/` git submodule | submodule commit + `.gitmodules` URL/branch |
| Dependency lockfile (ttfx) | `ttfx-src/Cargo.lock` | submodule commit |
| Rust toolchain | `rustc 1.98.0 (88d9e12ae 2026-08-18)` | documented here |

The submodule URL is `https://github.com/avillagran/ttfx` branch
`audio-background-vendor`, which carries the two engine commits this
plugin builds on top of upstream ttfx 0.3.2:

- `d6c4046` — audio-reactive color: global sgr_color hook
- `7e4a9b2` — audio-reactive thunderstorm (on_audio reference impl)

## Reproduce

```sh
# 1. Pristine clone at the commit you want to verify
git clone --depth 1 https://github.com/avillagran/omarchy-audio-background.git
cd omarchy-audio-background
git submodule update --init        # fetches ttfx-src at the pinned commit

# 2. Build (same toolchain as the CI: rustc 1.98.0)
cd bin/ttfx-bg-rs
cargo build --release

# 3. Compare against the bundled binary for your architecture
sha256sum target/release/ttfx-bg-rs
sha256sum ../ttfx-bg-rs-$(uname -m | sed 's/aarch64/aarch64/;s/x86_64/x86_64/')
```

The `release` profile (`lto = true`, `strip = true`, `codegen-units = 1`,
`panic` default) is defined in `bin/ttfx-bg-rs/Cargo.toml`; no extra
flags or environment variables are required.

## Continuous verification

`.github/workflows/verify-binaries.yml` performs steps 1-3 on every push
on native `ubuntu-24.04` (x86_64) and `ubuntu-24.04-arm` (aarch64)
runners and fails the build if the rebuilt binary's SHA-256 does not
exactly match the bundled one. A green run is proof that the committed
binary bytes are exactly what the committed source and lockfiles produce.

## Known non-determinism

None observed: with the lockfiles and toolchain above, rebuilds are
byte-identical across machines (verified on Arch Linux x86_64 and
aarch64). If you get a different hash, suspect a toolchain version
difference first (`rustc --version` must match exactly).
