#!/bin/sh
# Write one setting into the plugin state.json, preserving the rest.
# Usage:
#   write_state.sh running=1|0
#   write_state.sh audio=1|0
#   write_state.sh effect=matrix        (set active effect)
#   write_state.sh effect+fire          (enable effect in the rotation list)
#   write_state.sh effect-fire          (disable it)
#   write_state.sh intensity=0..10
#   write_state.sh byline=Some text     (intro byline; empty = default)
#   write_state.sh restart              (bump restart counter -> replay intro)
set -e
STATE="${HOME}/.config/omarchy/plugins/io.github.avillagran.omarchy-audio-background/state.json"
mkdir -p "$(dirname "$STATE")"

# current values (defaults match the Rust binary)
running="true"; audio="true"; effect="matrix"; intensity="5"; byline=""; restart="0"
effects="matrix rain wave bars donut fire starfield life"

if [ -f "$STATE" ]; then
  v=$(grep -o '"running"[^,}]*' "$STATE");   [ -n "$v" ] && running=$(echo "$v" | grep -o '\(true\|false\)')
  v=$(grep -o '"audio"[^,}]*' "$STATE");     [ -n "$v" ] && audio=$(echo "$v" | grep -o '\(true\|false\)')
  v=$(grep -o '"effect"[^,}]*' "$STATE");    [ -n "$v" ] && effect=$(echo "$v" | sed 's/.*:"\([^"]*\)".*/\1/')
  v=$(grep -o '"intensity"[^,}]*' "$STATE"); [ -n "$v" ] && intensity=$(echo "$v" | grep -o '[0-9]*')
  v=$(grep -o '"restart"[^,}]*' "$STATE");   [ -n "$v" ] && restart=$(echo "$v" | grep -o '[0-9]*')
  v=$(grep -o '"byline"[^,}]*' "$STATE");    [ -n "$v" ] && byline=$(echo "$v" | sed 's/.*:"\([^"]*\)".*/\1/')
  v=$(grep -o '"effects"[^]]*\]' "$STATE");  [ -n "$v" ] && effects=$(echo "$v" | sed 's/^"effects"\s*:\s*\[//; s/\]$//' | grep -o '"[a-z]*"' | tr -d '"' | tr '\n' ' ' | sed 's/ $//')
fi

arg="$1"
case "$arg" in
  running=*)  running=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  audio=*)    audio=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  intensity=*) intensity="${arg#*=}" ;;
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

printf '{"running":%s,"audio":%s,"effect":"%s","effects":[%s],"intensity":%s,"byline":"%s","restart":%s}\n' \
  "$running" "$audio" "$effect" "$arr" "$intensity" "$byline" "$restart" > "$STATE"
