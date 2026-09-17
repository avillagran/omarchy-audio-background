# Provenance of the prebuilt binaries

The plugin ships two prebuilt ELF executables so users don't need a Rust
toolchain:

- `bin/ttfx-bg-rs-x86_64` — canonical build produced by CI
  (`.github/workflows/rebuild-verify.yml`)
- `bin/ttfx-bg-rs-aarch64` — canonical build produced on the maintainer's
  aarch64 Arch system, bound by `build-manifest-aarch64.txt`

## Binding (both architectures)

| Input | Value | Pinned by |
|-------|-------|-----------|
| Plugin source | git tag/commit being shipped | git commit |
| ttfx engine (vendored) | `ttfx-src/` git submodule | exact gitlink in the tag |
| Wrapper lockfile | `bin/ttfx-bg-rs/Cargo.lock` | git commit |
| Engine lockfile | `ttfx-src/Cargo.lock` | submodule commit |
| Rust toolchain | `rustc 1.98.0 (88d9e12ae 2026-08-18)` via rustup | exact rustup version |
| Release profile | `lto=true`, `strip=true` | `bin/ttfx-bg-rs/Cargo.toml` |

The submodule URL is `https://github.com/avillagran/ttfx`, branch
`audio-background-vendor`. Always use the tag's exact submodule commit, not
the branch tip.

## x86_64 — canonical build in CI

The pinned build environment is fully defined and machine-verifiable:

- Container image: `archlinux@sha256:204e91950fd364961088a01773eee9012243b7e965fed42b1d82d12416190782`
- Package snapshot: `https://archive.archlinux.org/repos/2026/09/15` (daily
  snapshot; pacman mirror is rewritten to this URL inside the container)
- Toolchain: `rustup default 1.98.0`
- Source checkout path inside the container: `/oab/build` (the repo is
  mounted there; `scripts/repro-build.sh` remaps every compilable path to
  this canonical prefix)

`scripts/repro-build.sh` canonicalizes every path the compiler can embed
(crate, ttfx engine, target dir) to `/oab/build/...` via
`--remap-path-prefix`, so the binary never depends on the checkout location.

To verify the recorded hash, re-run the `rebuild-verify` workflow
(`workflow_dispatch`, default mode): it rebuilds from the exact committed
source and fails if the result differs from `SHA256SUMS.txt`. Each run also
uploads the rebuilt binary and the in-container `pacman -Q` manifest as
artifacts. To refresh the canonical binary intentionally, run the workflow
in `bootstrap` mode and commit the uploaded artifact.

Local reproduction of the CI build:

```sh
git clone https://github.com/avillagran/omarchy-audio-background.git
cd omarchy-audio-background
git checkout <tag>
git submodule update --init

docker run --rm -v "$PWD:/oab/build" -w /oab/build \
  archlinux@sha256:204e91950fd364961088a01773eee9012243b7e965fed42b1d82d12416190782 \
  bash -euo pipefail -c '
    echo "Server = https://archive.archlinux.org/repos/2026/09/15/\$repo/os/\$arch" > /etc/pacman.d/mirrorlist
    pacman -Syy --noconfirm
    pacman -S --needed --noconfirm base-devel rustup gtk4 gtk4-layer-shell vte4 pkgconf
    rustup default 1.98.0
    scripts/repro-build.sh
  '

grep ttfx-bg-rs-x86_64 SHA256SUMS.txt | sha256sum -c -
```

## aarch64 — manifest-bound canonical host

There is no official Arch Linux aarch64 port or package archive, so the
aarch64 binary cannot be rebuilt from a pinned distro snapshot. Its
canonical environment is the maintainer's aarch64 Arch system, bound by the
full package state in `build-manifest-aarch64.txt` (header records the
submodule gitlink and toolchain; body is `pacman -Q`).

Rebuild on the manifest-matched host:

```sh
git checkout <tag>
git submodule update --init
rustup default 1.98.0
scripts/repro-build.sh
grep ttfx-bg-rs-aarch64 SHA256SUMS.txt | sha256sum -c -
```

CI (`bind-aarch64` job) continuously verifies the recorded hash and the
manifest's presence and binding header.

## Verified evidence (2026-09-15)

These claims were exercised, not assumed:

- **Run-to-run determinism (aarch64, same host/path/toolchain):** two
  independent clean builds (default target dir; external `CARGO_TARGET_DIR`)
  produced byte-identical binaries (`c84d1c98…`).
- **Cross-checkout-path determinism:** a copy of the source at a different
  path does **not** byte-match. With the remap flags, all path strings are
  canonical (`/oab/build`, 54 occurrences in both binaries) and every
  section's content and address map is identical; only
  `.note.gnu.build-id` and `.rela.dyn` differ (1157 of 3248
  `R_AARCH64_RELATIVE` addends — internal link-order). This is why the
  canonical path `/oab/build` is part of the pinned environment.
- **Toolchain patch-level matters:** Arch's `rust 1.98.1` and the pinned
  `rustup 1.98.0` produce different binaries from identical source.
- **Rolling system libraries break reproduction over time:** the 0.0.8
  aarch64 binary (`3ce47f59…`, built ~2026-09-14 on its canonical host) no
  longer reproduces on that same host after routine system updates
  (today's clean rebuild: `c84d1c98…`). This is the exact failure mode the
  pinned CI environment and the aarch64 manifest close.

## Verification limits

- A `sha256sum -c SHA256SUMS.txt` check proves only that shipped files match
  their recorded hashes. The `rebuild-verify` workflow is the
  source-to-binary check; `check-binary-hashes.yml` is the cheap integrity
  gate against accidental corruption.
- aarch64 reproduction requires matching the full package manifest; there is
  no official aarch64 archive to pin against.
- On aarch64 the engine's release-only floating-point easing golden has a
  known bit-level mismatch, reproduced on unchanged source; the debug suite
  and band tests pass.
