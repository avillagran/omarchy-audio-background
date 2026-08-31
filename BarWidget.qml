import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "io.github.avillagran.omarchy-ttfx-background"

  readonly property string iconPath: Qt.resolvedUrl("icon.svg").toString().replace("file://", "")
  readonly property string stateFile: Quickshell.env("HOME") +
    "/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background/state.json"
  readonly property string writeState: Qt.resolvedUrl("bin/write_state.sh").toString().replace("file://", "")

  property bool running: true
  property string effect: "matrix"
  property int intensity: 5

  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  function open()    { if (panelLoader.item && panelLoader.item.open)    panelLoader.item.open() }
  function close()   { if (panelLoader.item && panelLoader.item.close)   panelLoader.item.close() }
  function togglePanel() { if (panelLoader.item && panelLoader.item.toggle) panelLoader.item.toggle() }

  function injectPanel() {
    var t = panelLoader.item
    if (!t) return
    if ("bar" in t) t.bar = root.bar
    if ("anchorItem" in t) t.anchorItem = root
    if ("hostWidget" in t) t.hostWidget = root
  }

  function refresh() { statusProc.running = true }

  Process {
    id: statusProc
    command: ["cat", root.stateFile]
    running: false
    stdout: StdioCollector {
      onStreamFinished: function() {
        try {
          var s = JSON.parse(text || "{}")
          if (typeof s.running === "boolean") root.running = s.running
          if (typeof s.effect === "string")   root.effect  = s.effect
          if (typeof s.intensity === "number") root.intensity = s.intensity
        } catch (e) {}
      }
    }
  }

  Timer { id: pollTimer; interval: 800; repeat: false; onTriggered: root.refresh() }
  Component.onCompleted: root.refresh()

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: { root.injectPanel(); Qt.callLater(root.injectPanel) }
  }

  // Icon (music note over rectangle) + live state label.
  RowLayout {
    spacing: 5
    Image {
      source: "file://" + root.iconPath
      sourceSize.width: 16; sourceSize.height: 16
      fillMode: Image.PreserveAspectFit
    }
    Text {
      text: root.running ? root.effect : "off"
      color: root.running ? "#00ffea" : "#888"
      font.pixelSize: 12
    }
  }

  MouseArea {
    anchors.fill: parent
    acceptedButtons: Qt.LeftButton | Qt.RightButton
    onClicked: function(m) {
      if (m.button === Qt.RightButton) root.refresh()
      else root.togglePanel()
    }
  }
}
