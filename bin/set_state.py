#!/usr/bin/env python3
"""Set fields in the audio-background state file.

Usage: set_state.py running=1 effect=bars rotate=1 words=Omarchy,Music
The service (Service.qml) polls this file every second and applies changes.
"""
import sys
import os
import json

STATE_DIR = os.path.expanduser(
    "~/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background")
STATE_FILE = os.path.join(STATE_DIR, "state.json")
os.makedirs(STATE_DIR, exist_ok=True)

DEFAULT = {"running": True, "effect": "bars", "rotate": True, "words": ["Omarchy"]}


def main():
    state = DEFAULT.copy()
    if os.path.exists(STATE_FILE):
        try:
            with open(STATE_FILE) as f:
                state.update(json.load(f))
        except Exception:
            pass
    for arg in sys.argv[1:]:
        if "=" not in arg:
            continue
        k, v = arg.split("=", 1)
        if k == "running":
            state["running"] = v.lower() in ("1", "true", "yes", "on")
        elif k == "rotate":
            state["rotate"] = v.lower() in ("1", "true", "yes", "on")
        elif k == "effect":
            state["effect"] = v
        elif k == "words":
            state["words"] = [w for w in v.split(",") if w]
    with open(STATE_FILE, "w") as f:
        json.dump(state, f)
    print("state:", json.dumps(state))


if __name__ == "__main__":
    main()
