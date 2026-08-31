# Omarchy Audio Background

Audio-reactive **animated desktop background** for Omarchy / Hyprland, controlled from the
Omarchy bar via a Quickshell plugin. It is a **real background layer** (Wayland layer-shell),
not a floating window: it lives *above* the wallpaper image and *below* app windows and the
bar, so it reads as a genuine living desktop.

## How it works

- `Service.qml` (plugin kind `service`) opens a `PanelWindow` on `WlrLayer.Bottom` with namespace
  `omarchy-ttfx-bg`. Omarchy's own wallpaper is on `WlrLayer.Background`, so this sits just above
  it; app windows and the bar render above it. Confirmed z-order: `Background(0) < ttfx-bg(1) < bar(2)`.
- A `PanelWindow` child `Image` displays `~/.cache/omarchy/ttfx_bg.png`, reloaded every frame.
- `render_bg.py` (pure python **stdlib**, no numpy/PIL) renders the current effect to that PNG at
  low resolution (320×180) and lets the compositor scale it up. It reads the latest audio frame
  from `/tmp/ttfx_bg_spectrum.json`, published by `analyzer_daemon.py` (Goertzel spectrum via
  `analyzer_light.Analyzer` + `parec` on the system monitor).
- `analyzer_daemon.py` spawns `parec`, runs the analyzer, and writes one JSON frame line per chunk
  (~30/s) to the temp file.

### Effects (rotate automatically)
`bars` (spectrum equalizer) → `wave` (waveform) → `radial` (radial spokes) → `rain`
(matrix-style falling columns). Rotation is every 12 s; disable with `rotate:false` and pick one
via `effect:"<name>"`. The visualizer is **always reactive** — an idle baseline animates even in
silence, and real audio raises the levels on top of it.

### Word list (config panel)
The config file `~/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background/state.json`
holds `words` (default `["Omarchy"]`). The `rain` effect tints its columns per word. Edit the list
to use your own words; the bar widget writes this file via `bin/set_state.py`.

State schema (`state.json`):
```json
{ "running": true, "effect": "bars", "rotate": true, "words": ["Omarchy"] }
```
- `running` — background on/off (bar widget left-click toggles it).
- `effect` — `"bars" | "wave" | "radial" | "rain"` (used when `rotate` is false).
- `rotate` — auto-cycle through effects.
- `words` — list of words feeding the text/rain effect (default `["Omarchy"]`).

## Control from the bar
`BarWidget.qml` (plugin kind `bar-widget`): left-click toggles `running`; right-click opens a menu
to pick the effect or toggle rotation. It writes `state.json` via `bin/set_state.py`. The service
polls that file every second, so changes apply within ~1 s.

No heavy deps (no numpy/PIL/pygame). Only `parec` (PipeWire/Pulse) + python stdlib.

> **Why not `ttfx`?** `ttfx` (the `Hypa.Ttfx` .NET build on this box) only animates on an
> interactive TTY. Fed via a pipe — even through a real PTY — it emits **0 bytes** and the window
> stays black, so it cannot serve as a continuous background. The effects here are our own stdlib
> renderers that replace ttfx's look (matrix/bars/wave/radial) while being continuous and
> audio-reactive.

> **Why a QML `Image` and not a QML `Canvas`?** On this Quickshell 0.3.1 build a `Canvas` inside a
> `PanelWindow` does not repaint from a `Timer`/`requestPaint()` (onPaint fires once at startup, then
> never again), so the canvas stays blank. A `PanelWindow` `Image` is proven to render in this shell
> (it is exactly how Omarchy's own wallpaper works), so we render frames in python and display them
> via the Image. This is reliable and low-CPU.

## Files
- `analyzer_light.py` — stdlib audio analyzer (Goertzel per band). `--test` synthesizes a sweep.
- `analyzer_daemon.py` — `parec` + analyzer → publishes `/tmp/ttfx_bg_spectrum.json`.
- `render_bg.py` — draws the current effect to `~/.cache/omarchy/ttfx_bg.png` (pure stdlib PNG).
- `Service.qml` — layer-shell background (`service` kind) that displays the PNG.
- `BarWidget.qml` + `manifest.json` — the Omarchy bar widget (toggle + effect menu).
- `bin/set_state.py` — writes `state.json` (running/effect/rotate/words) for the bar widget.
- `visualizer.py`, `medidor.py`, `bin/omarchy-ttfx-background` — earlier foot/ANSI prototypes
  (kept for reference; superseded by the layer-shell approach above).

## Requirements
- `parec` (PipeWire/Pulse). A Wayland session for the display; the analyzer `--test` works headless.

## Verify the analyzer (headless)
```sh
cd /home/avillagran/Work/omarchy-plugins/omarchy-ttfx-background
python3 analyzer_light.py --test
```
The dominant band (`^`) should climb 60Hz → … → 16000Hz.

## Install / enable
Local plugins are installed by **symlink** (Omarchy discovers `~/.config/omarchy/plugins/<id>`),
then enabled:
```sh
ln -sfn /home/avillagran/Work/omarchy-plugins/omarchy-ttfx-background \
       ~/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background
omarchy plugin enable io.github.avillagran.omarchy-ttfx-background --section right
omarchy-shell shell rescanPlugins   # if the plugin is not yet known
```
(`omarchy plugin add` is git-only and won't accept a local path.) After enabling, restart the shell
(`omarchy-restart-shell`) so the `service` kind loads. Left-click the widget to toggle the
background; right-click to pick the effect or toggle rotation.

## Monitor source — IMPORTANT (real bug found)
The default sink on this box is an **effect sink** (`audio_effect.j416-convolver`), not the
headphones device. A hardcoded `…Headphones__sink.monitor` captures **silence** because music is
routed through the effect sink first. `analyzer_daemon.py` resolves the monitor dynamically:
`pactl get-default-sink` + `.monitor` → `audio_effect.j416-convolver.monitor` (RUNNING while audio
plays). Override with `TTFX_MONITOR=<source>` if your routing differs. Verified: with the correct
monitor the visualizer tracks YouTube Music / any system audio (vol 0.4→1.0 in real time).

## Notes / limitations
- Resolution is low (320×180) and upscaled — intentionally blocky/retro and very low CPU. Bump
  `W, H` in `render_bg.py` for sharper output at higher cost.
- `Image` reload uses a cache-buster query each frame; at ~24 fps the background is smooth.
- The background sits on the Bottom layer, so maximized app windows cover it (that is the point of
  a real background). It shows on the bare desktop / behind translucent windows.
