// Omarchy Audio Background — bar widget controller.
// Left click: open the configuration panel (Panel.qml).
// The rendering is a real layer-shell background (bin/ttfx-bg-rs) driven by
// the audio analyzer. State (running/effect/intensity) lives in state.json,
// written via bin/set_state.py and polled by both this widget and the panel.
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import QtQuick.Controls
import qs.Ui
import qs.Commons

BarWidget {
  id: root
  moduleName: "io.github.avillagran.omarchy-ttfx-background"

  readonly property string binDir: (typeof manifest !== "undefined" && manifest.__sourceDir)
    ? manifest.__sourceDir.replace(/\/$/, "")
    : Qt.resolvedUrl(".").toString().replace("file://", "")
  readonly property string iconSvg: binDir + "/icon.svg"
  readonly property string setState: binDir + "/bin/set_state.py"
  readonly property string stateFile: Quickshell.env("HOME") +
    "/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background/state.json"

  property bool running: true
  property string effect: "matrix"
  property int intensity: 5

  // Forward panel lifecycle so clicking the pill opens/closes it.
  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  function open()    { if (panelLoader.item && panelLoader.item.open)    panelLoader.item.open() }
  function close()   { if (panelLoader.item && panelLoader.item.close)   panelLoader.item.close() }
  function toggle()  { if (root.opened) root.close() else root.open() }

  function refresh() { statusProc.running = true }

  Process {
    id: statusProc
    command: ["cat", root.stateFile]
    running: false
    stdout: StdioCollector {
      onStreamFinished: {
        try {
          var s = JSON.parse(text || "{}")
          if (typeof s.running === "boolean") root.running = s.running
          if (typeof s.effect === "string") root.effect = s.effect
          if (typeof s.intensity === "number") root.intensity = s.intensity
        } catch (e) {}
      }
    }
  }

  Timer { id: pollTimer; interval: 800; repeat: false; onTriggered: root.refresh() }
  Component.onCompleted: root.refresh()

  // The panel is loaded in-process so the pill can summon it.
  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
  }

  // --- UI ---
  RowLayout {
    spacing: 5
    Image {
      source: "file://" + root.iconSvg
      sourceSize.width: 16; sourceSize.height: 16
      fillMode: Image.PreserveAspectFit
    }
    Text {
      text: root.running ? (root.effect) : "off"
      color: root.running ? "#00ffea" : "#888"
      font.pixelSize: 12
    }
  }

  MouseArea {
    anchors.fill: parent
    acceptedButtons: Qt.LeftButton
    onClicked: root.toggle()
  }
}
