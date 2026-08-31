import Quickshell
import QtQuick
import Quickshell.Io
import Quickshell.Wayland

// Audio-reactive desktop background rendered to a PNG and shown on the Bottom
// layer-shell surface. The renderer (render_bg.py) draws a Matrix-rain style
// visualizer fed by the spectrum published by analyzer_daemon.py.
//
// NOTE: `ttfx` (Rust) cannot be used as the live background renderer here:
// Quickshell sanitizes LD_PRELOAD (so liblayer-shell-preload.so can't turn a
// terminal into a bottom layer from inside the plugin), foot won't become a
// layer-shell surface, and kitty only animates ttfx when launched with that
// preload outside Quickshell -- which a service process can't do. So we render
// an equivalent Matrix visualizer to a PNG, which is a genuine bottom-layer
// background above the wallpaper and below apps/bar, and reacts to audio.

PanelWindow {
    id: root
    WlrLayershell.namespace: "ttfx-bg"
    WlrLayershell.layer: WlrLayer.Bottom
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
    visible: true
    implicitWidth: 3456
    implicitHeight: 2234

    property string pluginDir: Qt.resolvedUrl(".").toString().replace("file://", "")
    readonly property string png: "/home/avillagran/.cache/omarchy/ttfx_bg.png"
    property int tick: 0

    Image {
        id: bg
        anchors.fill: parent
        fillMode: Image.Stretch
        cache: false
        source: "file://" + root.png + "#" + root.tick
    }

    Timer {
        interval: 33
        running: true
        onTriggered: root.tick++
    }

    // ---- audio analyzer (publishes the spectrum the renderer reads) ----
    Process {
        id: analyzerProc
        running: true
        command: ["python3", pluginDir + "/analyzer_daemon.py"]
    }

    // ---- png renderer (Matrix-rain visualizer) ----
    Process {
        id: renderProc
        running: true
        command: ["python3", pluginDir + "/render_bg.py"]
    }

    // kill both on unload so we don't leave stray processes
    Component.onDestruction: {
        renderProc.running = false
        analyzerProc.running = false
    }
}
