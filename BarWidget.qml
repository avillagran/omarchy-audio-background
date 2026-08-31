import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "io.github.avillagran.omarchy-audio-background"

  readonly property string stateFile: Quickshell.env("HOME") +
    "/.config/omarchy/plugins/io.github.avillagran.omarchy-audio-background/state.json"
  readonly property string writeState: Qt.resolvedUrl("bin/write_state.sh").toString().replace("file://", "")

  property bool running: true
  property string effect: "matrix"

  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  function open()    { if (panelLoader.item && panelLoader.item.open)    panelLoader.item.open() }
  function close()   { if (panelLoader.item && panelLoader.item.close)   panelLoader.item.close() }
  function togglePanel() { if (panelLoader.item && panelLoader.item.toggle) panelLoader.item.toggle() }

  function injectPanel() {
    var t = panelLoader.item
    if (!t) return
    if ("bar" in t) t.bar = root.bar
    if ("anchorItem" in t) t.anchorItem = button
    if ("hostWidget" in t) t.hostWidget = root
  }

  // The bar is assigned asynchronously after the widget loads; re-inject the
  // panel then, or it keeps bar=null and the popup can't anchor/open.
  onBarChanged: injectPanel()

  function refresh() { statusProc.running = true }

  // Right click: restart the background (bump restart counter -> replays intro).
  function restartBackground() {
    Quickshell.execDetached(["sh", root.writeState, "restart"])
  }

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
        } catch (e) {}
      }
    }
  }

  Timer { id: pollTimer; interval: 900; repeat: false; onTriggered: root.refresh() }
  Component.onCompleted: root.refresh()

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: { root.injectPanel(); Qt.callLater(root.injectPanel) }
  }

  // Theme-colored, optically centered glyph (white/black follows the bar).
  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: "♪"
    active: !root.running
    onPressed: function(b) {
      if (!root.bar) return
      if (b === Qt.RightButton) root.restartBackground()
      else root.togglePanel()
    }
  }
}
