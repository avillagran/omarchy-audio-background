#!/bin/sh
# Write one setting into the plugin state.json, preserving the rest.
# Usage:
#   write_state.sh running=1|0
#   write_state.sh audio=1|0
#   write_state.sh effect=matrix        (set active effect)
#   write_state.sh effect+fire          (enable effect in the rotation list)
#   write_state.sh effect-fire          (disable it)
#   write_state.sh intensity=0..10
#   write_state.sh intro_size=1..16     (intro title text scale)
#   write_state.sh show_fps=1|0         (FPS counter overlay)
#   write_state.sh byline=Some text     (intro byline; empty = default)
#   write_state.sh restart              (bump restart counter -> replay intro)
set -e
STATE="${HOME}/.config/omarchy/plugins/io.github.avillagran.omarchy-audio-background/state.json"
mkdir -p "$(dirname "$STATE")"

# current values (defaults match the Rust binary)
running="true"; audio="true"; effect="matrix"; intensity="5"; byline=""; restart="0"; intro_size="5"; show_fps="false"
effects="matrix rain wave bars donut fire starfield life"

if [ -f "$STATE" ]; then
  # `|| true` on every read: under `set -e`, a grep that finds no key returns
  # 1 and the `v=$(...)` assignment would abort the whole script before any
  # write. state.json may legitimately lack newer keys (show_fps, intro_size).
  v=$(grep -o '"running"[^,}]*' "$STATE" || true);   [ -n "$v" ] && running=$(echo "$v" | grep -o '\(true\|false\)')
  v=$(grep -o '"audio"[^,}]*' "$STATE" || true);     [ -n "$v" ] && audio=$(echo "$v" | grep -o '\(true\|false\)')
  v=$(grep -o '"show_fps"[^,}]*' "$STATE" || true);  [ -n "$v" ] && show_fps=$(echo "$v" | grep -o '\(true\|false\)')
  v=$(grep -o '"effect"[^,}]*' "$STATE" || true);    [ -n "$v" ] && effect=$(echo "$v" | sed 's/.*:"\([^"]*\)".*/\1/')
  v=$(grep -o '"intensity"[^,}]*' "$STATE" || true); [ -n "$v" ] && intensity=$(echo "$v" | grep -o '[0-9]\+')
  v=$(grep -o '"restart"[^,}]*' "$STATE" || true);   [ -n "$v" ] && restart=$(echo "$v" | grep -o '[0-9]\+')
  v=$(grep -o '"intro_size"[^,}]*' "$STATE" || true); [ -n "$v" ] && intro_size=$(echo "$v" | grep -o '[0-9]\+')
  v=$(grep -o '"byline"[^,}]*' "$STATE" || true);    [ -n "$v" ] && byline=$(echo "$v" | sed 's/.*:"\([^"]*\)".*/\1/')
  v=$(grep -o '"effects"[^]]*\]' "$STATE" || true);  [ -n "$v" ] && effects=$(echo "$v" | sed 's/^"effects"\s*:\s*\[//; s/\]$//' | grep -o '"[a-z]*"' | tr -d '"' | tr '\n' ' ' | sed 's/ $//')
fi

arg="$1"
case "$arg" in
  running=*)  running=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  audio=*)    audio=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  show_fps=*) show_fps=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  intensity=*) intensity="${arg#*=}" ;;
  intro_size=*) intro_size="${arg#*=}" ;;
  effect=*)   effect="${arg#*=}" ;;
  byline=*)   byline="${arg#*=}" ;;
  restart)    restart=$((restart + 1)) ;;
  effect+*)
    name="${arg#*+}"
    case " $effects " in *" $name "*) ;; *) effects="$effects $name" ;; esac ;;
  effect-*)
    name="${arg#*-}"
    effects=$(echo " $effects " | sed "s/ $name / /" | xargs) ;;
esac

# never let the effects list go empty
[ -z "$effects" ] && effects="$effect"

# emit JSON array for effects
arr=$(echo "$effects" | tr ' ' '\n' | sed 's/.*/"&"/' | paste -sd, -)

printf '{"running":%s,"audio":%s,"show_fps":%s,"effect":"%s","effects":[%s],"intensity":%s,"byline":"%s","restart":%s,"intro_size":%s}\n' \
  "$running" "$audio" "$show_fps" "$effect" "$arr" "$intensity" "$byline" "$restart" "$intro_size" > "$STATE"
