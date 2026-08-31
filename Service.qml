import Quickshell
import QtQuick
import Quickshell.Io

// Audio-reactive desktop background — plugin launcher.
//
// This plugin does NOT render anything itself. It starts the prebuilt Rust
// binary (bin/ttfx-bg-launch.sh -> bin/ttfx-bg-rs-<arch>), which opens one
// gtk4-layer-shell window per monitor and draws the effect inside an embedded
// Vte terminal. The binary owns the layer-shell surface; the plugin only keeps
// it alive for the lifetime of the shell.
//
// No build step: the binaries are committed, so `omarchy plugin add` is enough.
Item {
  id: root
  property string pluginDir: Quickshell.pluginDir

  Process {
    id: bgProc
    running: true
    command: [pluginDir + "/bin/ttfx-bg-launch.sh"]
    // Inherits WAYLAND_DISPLAY / HYPRLAND_INSTANCE_SIGNATURE from the session.
  }
}
