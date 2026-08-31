# Omarchy Audio Background

Animated, audio-reactive **desktop background** for Omarchy / Hyprland, controlled from
the Omarchy bar via a Quickshell plugin. It is a **real background layer** (Wayland
layer-shell), not a floating window: it lives *below* app windows and the bar, so it
reads as a genuine living desktop.

> **Status: 100% Rust.** The old Python/PNG pipeline (`render_bg.py`,
> `analyzer_daemon.py`, etc.) is gone. The background is now rendered entirely in Rust
> (gtk4-layer-shell + Vte + a hand-rolled renderer). See **Roadmap** for the next step:
> integrating [`ttfx`](https://github.com/omacom/ttfx) as the effects engine.

## How it works

- A single Rust binary (`bin/ttfx-bg-rs`) opens **one gtk4-layer-shell window per
  connected monitor**, each anchored to `WlrLayer.Bottom`, so the background covers
  *every* screen (internal + external). Hyprland re-adds a monitor on a resolution
  change, which triggers a rebuild so each layer keeps matching the new geometry.
- Inside each layer we embed a **Vte terminal** and draw the effect directly to it.
  (ttfx's own binary freezes inside an embedded Vte — proven — but our own renderer
  animates fine, so we use our renderer as the continuous background.)
- A `glib` timer polls `state.json` (written by the config panel) every 700 ms. If
  `running` / `effect` / `intensity` changed, the layers are rebuilt and the renderer
  subprocesses restarted with the new settings — no restart of the binary needed.
- The renderer is launched as a child of the Vte PTY (`ttfx-bg-rs --matrix COLS ROWS
  --effect NAME --intensity N`). Killing the parent's renderers on rebuild avoids
  orphaned processes.

### Effects

`matrix` (green, default) · `rain` (cyan) · `wave` (magenta) · `bars` (yellow).
Each is a color scheme of the same column-rain renderer; `intensity` (0–10) scales
speed and trail length. These are placeholders until the real `ttfx` effects land.

### Configuration (`state.json`)

`~/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background/state.json`:

```json
{ "running": true, "effect": "matrix", "intensity": 5 }
```

- `running` — background on/off (toggle in the config panel / bar widget).
- `effect`  — `matrix` | `rain` | `wave` | `bars`.
- `intensity` — 0–10, animation speed / trail length.

## Control from the bar

- `BarWidget.qml` — bar icon (music-note over rectangle). Left-click opens the config
  panel; it writes `state.json`.
- `Panel.qml` — the configuration panel (toggle on/off, effect picker, intensity
  slider). Also writes `state.json`.
- The binary polls that file, so changes apply within ~1 s.

## Running

The background is meant to run as a **systemd user service** (so it survives agent /
shell restarts and auto-restarts on failure):

```sh
cp ttfx-bg.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ttfx-bg.service
```

The service inherits the graphical-session environment (`WAYLAND_DISPLAY`,
`HYPRLAND_INSTANCE_SIGNATURE`, `DBUS_SESSION_BUS_ADDRESS`) from `systemctl --user
show-environment`. To run manually for debugging:

```sh
cd bin/ttfx-bg-rs
cargo build --release
WAYLAND_DISPLAY=wayland-1 \
HYPRLAND_INSTANCE_SIGNATURE=<from `echo $HYPRLAND_INSTANCE_SIGNATURE`> \
  ./target/release/ttfx-bg-rs
```

## Files

- `bin/ttfx-bg-rs/src/main.rs` — layer-shell windows (one per monitor), Vte host,
  `state.json` polling, renderer lifecycle.
- `bin/ttfx-bg-rs/Cargo.toml` — gtk4, gtk4-layer-shell, vte4, rand, anyhow.
- `manifest.json` — plugin manifest (kinds: `bar-widget`, `service`, `panel`).
- `BarWidget.qml` — bar icon / opens config panel.
- `Panel.qml` — configuration panel (toggle, effect, intensity).
- `icon.svg` — plugin icon (music note over rectangle).
- `ttfx-bg.service` — systemd user service unit.
- `ttfx-src/` — git submodule: [`omacom/ttfx`](https://github.com/omacom/ttfx)
  (upstream effects engine, not yet wired in — see Roadmap).

## Requirements

- Rust toolchain (rustup), plus `gtk4`, `gtk4-layer-shell`, `vte4` dev headers
  (`pkg-config` path must include them; on this box they live under
  `~/.local/share/pkgconfig`).
- A Wayland session (Hyprland). CPU stays well under 5% (idle animation in Vte).

## Roadmap

The goal is to use **ttfx** as the effects engine, vendored as a git submodule so it
can be updated from upstream:

1. **Bridge ttfx (Step 1).** Expose `ttfx`'s effect engine as a library (or a thin
   wrapper crate) and call it from `bin/ttfx-bg-rs` instead of the hand-rolled
   renderer — replacing the placeholder `matrix`/`rain`/`wave`/`bars` with ttfx's
   real matrix / rain / fireworks / etc. effects. `ttfx-src/` is already a submodule
   at `omacom/ttfx`; the bridge is the missing piece.
2. **Audio-reactive (Step 2).** Feed system audio into the engine (PulseAudio /
   PipeWire monitor capture) so the animations react to music, as the original
   concept intended.

## Install / enable the plugin

Install from this repository with the Omarchy CLI (it clones into
`~/.config/omarchy/plugins/<id>` automatically — no manual symlink needed):

```sh
omarchy plugin add https://github.com/avillagran/omarchy-ttfx-background.git --enable --yes
```

If the plugin is already on disk but the shell hasn't picked it up, force a reload:

```sh
omarchy-shell shell rescanPlugins
```

After enabling, restart the shell (`omarchy-restart-shell`) so the `service` / `panel`
kinds load. Left-click the bar widget to open the config panel.

> For local development you can symlink your working copy into the plugins dir:
> ```sh
> ln -sfn "$PWD" ~/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background
> ```
> (run from the repo root, so `$PWD` resolves to your checkout — never hardcode a
> user home path).

## Notes / limitations

- The background sits on the Bottom layer, so maximized app windows cover it (that is
  the point of a real background). It shows on the bare desktop / behind translucent
  windows.
- Font size is computed in physical pixels divided by the monitor scale factor, so the
  glyph grid scales with the screen resolution (dense on 4K, coarser on small panels)
  instead of a few giant characters.
- Renderer effects are placeholders until the ttfx bridge (Step 1) lands.
