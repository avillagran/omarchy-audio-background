# Provenance of the prebuilt binaries

The plugin ships two prebuilt ELF executables so users don't need a Rust
toolchain:

- `bin/ttfx-bg-rs-aarch64`
- `bin/ttfx-bg-rs-x86_64`

## Canonical builder: the CI workflow

**The bundled binaries are always the output of
`.github/workflows/build-binaries.yml`.** That workflow is the complete,
auditable recipe: it checks out the exact commit (with the `ttfx-src`
submodule at its pinned commit), installs the pinned toolchain
(`rustc 1.98.0` via rustup, declared as `TOOLCHAIN` in the workflow),
installs the GTK4/VTE system dependencies, and builds the wrapper in
release mode from `bin/ttfx-bg-rs` with the committed `Cargo.lock`.

Every run builds twice from a clean target directory and **fails unless
both builds produce the identical SHA-256**, so the published binaries
are provably deterministic for that source tree. The resulting hashes are
committed to `SHA256SUMS.txt` in the same push, alongside the binaries
themselves, by `github-actions[bot]`.

To verify that a bundled binary matches the source at commit `X`:

1. Confirm the binary was last touched in a
   `build: canonical ...` commit by `github-actions[bot]`, whose parent
   is `X` (or that the workflow ran on `X`).
2. Re-run the workflow (`workflow_dispatch`) or push any change under
   `bin/ttfx-bg-rs/` — the rebuilt hash must equal the one in
   `SHA256SUMS.txt`, or the job fails.

## What goes into a binary

| Input | Location | Pinned by |
|-------|----------|-----------|
| Background wrapper source | `bin/ttfx-bg-rs/src/main.rs` | git commit |
| Wrapper dependency lockfile | `bin/ttfx-bg-rs/Cargo.lock` | git commit |
| ttfx effects engine (vendored) | `ttfx-src/` git submodule | submodule commit + `.gitmodules` URL/branch |
| ttfx dependency lockfile | `ttfx-src/Cargo.lock` | submodule commit |
| Rust toolchain | `rustc 1.98.0` | `TOOLCHAIN` env in the workflow |
| Release profile (lto, strip, codegen-units) | `bin/ttfx-bg-rs/Cargo.toml` | git commit |

The submodule URL is `https://github.com/avillagran/ttfx` branch
`audio-background-vendor`, which carries the two engine commits this
plugin builds on top of upstream ttfx 0.3.2:

- `d6c4046` — audio-reactive color: global sgr_color hook
- `7e4a9b2` — audio-reactive thunderstorm (on_audio reference impl)

## Reproducing locally

The workflow environment is Ubuntu 24.04 (x86_64 and arm64 runners) with
the packages listed above. Any drift in compiler or linker compared to
that environment can change the bytes, which is exactly why the CI
workflow — not a developer laptop — is the canonical builder. For local
reproduction of the *exact* bundled bytes, re-run the workflow rather
than building by hand. For a local build that is functionally identical
but not necessarily byte-identical:

```sh
git clone https://github.com/avillagran/omarchy-audio-background.git
cd omarchy-audio-background
git submodule update --init
cd bin/ttfx-bg-rs
cargo build --release          # needs rust 1.98.0 + gtk4 + vte dev libs
```
