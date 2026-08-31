// Omarchy Audio Background — bar widget controller.
// Left click: toggle the audio-reactive background on/off.
// Right click: pick the effect (bars / wave / radial / rain) or toggle rotation.
// The rendering is a real layer-shell background (Service.qml, WlrLayer.Background)
// driven by the audio analyzer. State (running/effect/words/rotate) lives in
// state.json, written here via bin/set_state.py and read by the service.
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
  readonly property string setState: binDir + "/bin/set_state.py"
  readonly property string stateFile: Quickshell.env("HOME") +
    "/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background/state.json"

  property bool running: true
  property string effect: "bars"
  readonly property var effects: ["bars", "wave", "radial", "rain"]

  function refresh() { statusProc.running = true }

  function setRunning(v) {
    root.running = v
    Quickshell.execDetached(["python3", root.setState, "running=" + (v ? "1" : "0")])
    pollTimer.restart()
  }
  function toggle() { root.setRunning(!root.running) }

  function pickEffect(e) {
    root.effect = e
    Quickshell.execDetached(["python3", root.setState, "effect=" + e])
    pollTimer.restart()
  }
  function toggleRotate() {
    Quickshell.execDetached(["python3", root.setState, "rotate=" + (root.rotate ? "0" : "1")])
    root.rotate = !root.rotate
    pollTimer.restart()
  }
  property bool rotate: true

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
          if (typeof s.rotate === "boolean") root.rotate = s.rotate
        } catch (e) {}
      }
    }
  }

  Timer {
    id: pollTimer
    interval: 800
    repeat: false
    onTriggered: root.refresh()
  }

  Component.onCompleted: root.refresh()

  // --- UI ---
  RowLayout {
    spacing: 4
    Text {
      text: root.running ? "▮" : "▯"
      color: root.running ? "#00ffea" : "#888"
      font.pixelSize: 16
      font.bold: true
    }
    Label {
      text: root.running ? ("bg:" + root.effect) : "bg"
      color: root.running ? "#00ffea" : "#aaa"
      font.pixelSize: 12
    }
  }

  MouseArea {
    anchors.fill: parent
    acceptedButtons: Qt.LeftButton | Qt.RightButton
    onClicked: function (mouse) {
      if (mouse.button === Qt.RightButton) effectMenu.open()
      else root.toggle()
    }
  }

  Menu {
    id: effectMenu
    title: "Background"
    Repeater {
      model: root.effects
      MenuItem { text: modelData; onTriggered: root.pickEffect(modelData) }
    }
    MenuSeparator { }
    MenuItem {
      text: root.rotate ? "Rotation: ON" : "Rotation: OFF"
      onTriggered: root.toggleRotate()
    }
  }
}
