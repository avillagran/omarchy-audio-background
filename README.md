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

`matrix` (green rain) · `rain` (cyan rain) · `wave` (sine) · `bars` (spectrum) ·
`donut` (3D torus) · `fire` (doom fire) · `starfield` · `life` (Conway). Enable
any subset in the panel; with more than one enabled the parent rotates the active
effect every 20 s.

### Audio reactivity (per-band, like the reference implementation)

The renderer spawns `parec` on the default sink monitor and runs a Goertzel
per-band analysis (24 bands, log-spaced 60 Hz–10 kHz) with rolling-peak adaptive
normalization — the same approach as the reference analyzer that "reacts
correctly". Each effect consumes the shared `AudioState` (volume + beat + bands):
- **matrix/rain**: each rain column maps to a frequency band — band energy raises
  that column's fall speed and brightness (a "rain equalizer").
- **bars**: a true spectrum equalizer (bar = band energy).
- **wave/donut/fire/starfield/life**: volume modulates amplitude / spin / fuel /
  speed / reseed density.
Global volume also increases spawn density ("caudal") and frame rate. No blinking —
brightness follows band energy smoothly.

### Configuration (`state.json`)

`~/.config/omarchy/plugins/io.github.avillagran.omarchy-audio-background/state.json`:

```json
{
  "running": true, "audio": true, "effect": "matrix",
  "effects": ["matrix","rain","wave","bars","donut","fire","starfield","life"],
  "intensity": 5, "byline": "", "restart": 0, "intro_size": 5
}
```

- `running` — background on/off.
- `audio` — react to system audio on/off.
- `effect` — active effect (used when only one is enabled).
- `effects` — enabled set; >1 rotates every 20 s.
- `intensity` — 0–10, animation speed / trail length.
- `byline` — intro signature text; empty = default `By x.com/@avillagran`.
- `restart` — counter; bump to replay the intro (bar-widget right-click).
- `intro_size` — 1–16, intro title text scale. On dense HiDPI grids use a higher
  value for a readable boot splash; centered horizontally and vertically.

## Control from the bar

- `BarWidget.qml` — centered, theme-colored ♪ icon (white/black follows the bar).
  Left-click opens the config panel; **right-click restarts the background**
  (replays the intro).
- `Panel.qml` — configuration panel: Enabled, React to audio, per-effect toggle
  list, intensity slider, intro text size slider, intro byline field.
- The binary polls `state.json` every 700 ms, so changes apply within ~1 s.

## Running

When the plugin is enabled, its `service` kind starts the background automatically —
no extra steps. Just `omarchy plugin add … --enable` and restart the shell.

Optionally, for a background that survives shell restarts and auto-restarts on crash
independent of the shell, run it as a **systemd user service** instead. Use this OR
the built-in plugin service, **not both** (two instances would stack and double CPU):

```sh
cp ttfx-bg.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ttfx-bg.service
```

The unit's `ExecStart` uses `%h` (the installing user's home) and the
`bin/ttfx-bg-launch.sh` arch launcher, so it works on any install with no edits. It
inherits the graphical-session environment (`WAYLAND_DISPLAY`,
`HYPRLAND_INSTANCE_SIGNATURE`, `DBUS_SESSION_BUS_ADDRESS`) from `systemctl --user
show-environment`.

To build and run the renderer manually for debugging:

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
- `manifest.json` — plugin manifest (kinds: `bar-widget`, `service`). The bar widget
  opens the config panel (`Panel.qml`) via an internal `Loader`; do NOT add a `panel`
  kind — the shell treats any plugin with a `panel`/`overlay`/`menu` kind as a pure
  panel and would then NOT load the bar widget (the bar icon would disappear).
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

**ttfx as the effects engine.** The current renderer is hand-rolled (matrix, rain,
wave, bars, donut, fire, starfield, life) and is **already audio-reactive** — it
captures the default sink monitor with `parec` and drives every effect from a
per-band Goertzel analysis (see "Audio reactivity" above). The remaining step is to
vendor [`ttfx`](https://github.com/omacom/ttfx) as the effects engine — replacing the
hand-rolled effects with ttfx's richer set while keeping the same audio drive.
`ttfx-src/` is already a submodule at `omacom/ttfx`; the bridge is the missing piece.

## Install / enable the plugin

Install from this repository with the Omarchy CLI (it clones into
`~/.config/omarchy/plugins/<id>` automatically — no manual symlink needed):

```sh
omarchy plugin add https://github.com/avillagran/omarchy-audio-background.git --enable --yes
```

If the plugin is already on disk but the shell hasn't picked it up, force a reload:

```sh
omarchy-shell shell rescanPlugins
```

After enabling, restart the shell (`omarchy-restart-shell`) so the `service` / `panel`
kinds load. Left-click the bar widget to open the config panel.

> For local development you can symlink your working copy into the plugins dir:
> ```sh
> ln -sfn "$PWD" ~/.config/omarchy/plugins/io.github.avillagran.omarchy-audio-background
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
