#!/bin/sh
# Write one key=value pair into the plugin state.json, preserving the others.
# Usage: write_state.sh running=1 | effect=rain | intensity=5
set -e
STATE="${HOME}/.config/omarchy/plugins/io.github.avillagran.omarchy-audio-background/state.json"
mkdir -p "$(dirname "$STATE")"

key="${1%%=*}"; val="${1#*=}"
running="true"; effect="matrix"; intensity="5"

if [ -f "$STATE" ]; then
  r=$(grep -o '"running"[^,}]*' "$STATE"); [ -n "$r" ] && running=$(echo "$r" | grep -o '\(true\|false\)')
  e=$(grep -o '"effect"[^,}]*' "$STATE");  [ -n "$e" ] && effect=$(echo "$e"  | sed 's/.*:"\([^"]*\)".*/\1/')
  i=$(grep -o '"intensity"[^,}]*' "$STATE");[ -n "$i" ] && intensity=$(echo "$i" | grep -o '[0-9]*')
fi

case "$key" in
  running)   running="$val" ;;
  effect)    effect="$val" ;;
  intensity) intensity="$val" ;;
esac

printf '{"running":%s,"effect":"%s","intensity":%s}\n' "$running" "$effect" "$intensity" > "$STATE"
