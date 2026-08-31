#!/usr/bin/env bash
# Launch the ttfx matrix background as a real layer-shell surface.
# kitty is forced onto the Bottom layer via liblayer-shell-preload.so so it sits
# above the wallpaper image and below app windows and the bar -- a genuine
# background, not an XDG window.
#
# Inside kitty we run ttfx_tty.py, which gives `ttfx matrix` a REAL PTY so it
# animates, fed a music-reactive Matrix-rain stream. (A plain `feed | ttfx`
# pipe breaks ttfx's TTY detection, so the PTY is required; ttfx_tty.py also
# bridges ttfx's pty output back to stdout so kitty displays it.)
DIR="$(cd "$(dirname "$0")/.." && pwd)"
echo "launcher start pid $$ LD_PRELOAD=$LD_PRELOAD" >> /tmp/ttfx_bg_svc.log
exec 2>>/tmp/ttfx_bg_svc.log 1>>/tmp/ttfx_bg_svc.log
# ensure the display is available even when launched from Quickshell's service
export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-1}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/1001}"
export LD_PRELOAD=/usr/lib/liblayer-shell-preload.so
LAYER_LAYER=bottom
LAYER_NAMESPACE=ttfxmatrix
LAYER_KEYBOARD=n
LAYER_ANCHOR=ltrb
LAYER_WIDTH=3456
LAYER_HEIGHT=2234
exec /usr/bin/kitty --class omarchy-ttfx-bg -o font_size=26 \
  sh -c "python3 $DIR/ttfx_tty.py" 2>>/tmp/ttfx_bg_svc.log
