import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "io.github.avillagran.omarchy-ttfx-background"
  ipcTarget: "io.github.avillagran.omarchy-ttfx-background"
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null
  readonly property var barIdentity: hostWidget || root

  readonly property string stateFile: Quickshell.env("HOME") +
    "/.config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background/state.json"
  readonly property string writeState: Qt.resolvedUrl("bin/write_state.sh").toString().replace("file://", "")
  readonly property string iconPath: Qt.resolvedUrl("icon.svg").toString().replace("file://", "")

  property bool running: true
  property string effect: "matrix"
  property int intensity: 5
  readonly property var effects: ["matrix", "rain", "wave", "bars"]
  property string label: running ? "♪" : "off"

  readonly property bool opened: root.controller && root.controller.visible === true
  function open() { root.controller.show() }
  function close() { root.controller.hide() }
  function toggle() {
    if (root.opened) { root.close() } else { root.open() }
  }
  function openFromHotkey() { root.open() }
  property bool popoutSwitchClosing: false
  function closeForPopoutSwitch() { root.close() }

  function refresh() { statusProc.running = true }
  function setRunning(v)   { root.running = v;  Quickshell.execDetached(["sh", root.writeState, "running="   + (v ? 1 : 0)]); pollTimer.restart() }
  function pickEffect(e)   { root.effect  = e;  Quickshell.execDetached(["sh", root.writeState, "effect="    + e]);               pollTimer.restart() }
  function setIntensity(v) { root.intensity = v; Quickshell.execDetached(["sh", root.writeState, "intensity=" + v]); }

  Component.onCompleted: root.refresh()

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
  Timer { id: pollTimer; interval: 600; repeat: false; onTriggered: root.refresh() }

  IpcHandler {
    target: root.ipcTarget
    function open(): void  { root.open() }
    function close(): void { root.close() }
    function toggle(): void { root.toggle() }
  }

  KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root.barIdentity
    bar: root.bar
    open: root.opened
    contentWidth: panel.fittedContentWidth(Style.space(360))
    focusTarget: keyCatcher

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(d) { if (root.bar) root.bar.switchPanelFrom(root.barIdentity, d) }

      Column {
        width: parent.width
        spacing: Style.space(12)
        leftPadding: Style.space(16); rightPadding: Style.space(16)
        topPadding: Style.space(14);  bottomPadding: Style.space(14)

        Row {
          spacing: Style.space(10)
          Image {
            source: "file://" + root.iconPath
            width: Style.font.title; height: Style.font.title
            fillMode: Image.PreserveAspectFit
            anchors.verticalCenter: parent.verticalCenter
          }
          Text {
            text: "Audio Background"
            color: root.bar ? root.bar.foreground : "#fff"
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.title
            font.bold: true
            anchors.verticalCenter: parent.verticalCenter
          }
        }

        Row {
          spacing: Style.space(10)
          Text {
            text: "Enabled"
            color: root.bar ? root.bar.foreground : "#fff"
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            anchors.verticalCenter: parent.verticalCenter
          }
          Switch {
            checked: root.running
            onToggled: root.setRunning(checked)
            anchors.verticalCenter: parent.verticalCenter
          }
        }

        Text {
          text: "EFFECT"
          color: Qt.darker(root.bar ? root.bar.foreground : "#fff", 1.5)
          font.family: root.bar ? root.bar.fontFamily : "sans"
          font.pixelSize: Style.font.bodySmall
          font.letterSpacing: 1
        }
        Flow {
          width: parent.width
          spacing: Style.space(8)
          Repeater {
            model: root.effects
            Button {
              text: modelData
              highlighted: root.effect === modelData
              onClicked: root.pickEffect(modelData)
            }
          }
        }

        Text {
          text: "INTENSITY"
          color: Qt.darker(root.bar ? root.bar.foreground : "#fff", 1.5)
          font.family: root.bar ? root.bar.fontFamily : "sans"
          font.pixelSize: Style.font.bodySmall
          font.letterSpacing: 1
        }
        Slider {
          width: parent.width
          from: 0; to: 10; stepSize: 1
          value: root.intensity
          onMoved: root.setIntensity(value)
        }
      }
    }
  }
}
