import QtQuick
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "io.github.avillagran.omarchy-ttfx-background"

  readonly property string stateFile: Quickshell.env("HOME") +
    "/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background/state.json"
  readonly property string writeState: Qt.resolvedUrl("bin/write_state.sh").toString().replace("file://", "")

  property bool running: true
  property string effect: "matrix"
  property int intensity: 5

  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  function open()    { if (panelLoader.item && panelLoader.item.open)    panelLoader.item.open() }
  function close()   { if (panelLoader.item && panelLoader.item.close)   panelLoader.item.close() }
  function toggle()  { if (root.opened) root.close() else root.open() }
  function togglePanel() { if (panelLoader.item && panelLoader.item.toggle) panelLoader.item.toggle() }

  function injectPanel() {
    var t = panelLoader.item
    if (!t) return
    if ("bar" in t) t.bar = root.bar
    if ("anchorItem" in t) t.anchorItem = button
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

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.running ? "♪" : "♪̶"
    slotSize: Style.bar.statusSlot
    onPressed: function(b) {
      if (!root.bar) return
      if (b === Qt.RightButton) root.refresh()
      else root.togglePanel()
    }
  }
}
