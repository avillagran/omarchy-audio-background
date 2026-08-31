import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "io.github.avillagran.omarchy-audio-background"
  ipcTarget: "io.github.avillagran.omarchy-audio-background"
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null
  readonly property var barIdentity: hostWidget || root

  readonly property string stateFile: Quickshell.env("HOME") +
    "/.config/omarchy/plugins/io.github.avillagran.omarchy-audio-background/state.json"
  readonly property string writeState: Qt.resolvedUrl("bin/write_state.sh").toString().replace("file://", "")

  property bool running: true
  property bool audio: true
  property string effect: "matrix"
  property var effects: ["matrix", "rain", "wave", "bars", "donut", "fire", "starfield", "life"]
  property int intensity: 5
  property string byline: ""
  property string label: running ? "♪" : "off"

  readonly property var allEffects: ["matrix", "rain", "wave", "bars", "donut", "fire", "starfield", "life"]

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

  function write(arg) { Quickshell.execDetached(["sh", root.writeState, arg]) }
  function setRunning(v)   { root.running = v;   write("running="   + (v ? "1" : "0")); }
  function setAudio(v)     { root.audio = v;     write("audio="     + (v ? "1" : "0")); }
  function pickEffect(e)   { root.effect = e;    write("effect="    + e); }
  function setIntensity(v) { root.intensity = v; write("intensity=" + v); }
  function setByline(t)    { root.byline = t;    write("byline="    + t); }
  function toggleEffect(e, on) {
    var list = root.effects.slice()
    var i = list.indexOf(e)
    if (on && i < 0) list.push(e)
    if (!on && i >= 0) list.splice(i, 1)
    if (list.length === 0) return // never empty
    root.effects = list
    write("effect" + (on ? "+" : "-") + e)
  }

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
          if (typeof s.audio === "boolean")   root.audio = s.audio
          if (typeof s.effect === "string")   root.effect = s.effect
          if (Array.isArray(s.effects) && s.effects.length) root.effects = s.effects
          if (typeof s.intensity === "number") root.intensity = s.intensity
          if (typeof s.byline === "string")   root.byline = s.byline
        } catch (e) {}
      }
    }
  }

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
    contentWidth: panel.fittedContentWidth(Style.space(380))
    focusTarget: keyCatcher

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(d) { if (root.bar) root.bar.switchPanelFrom(root.barIdentity, d) }

      ColumnLayout {
        width: parent.width
        spacing: Style.space(12)
        // leftPadding/rightPadding handled by the panel content margins
        anchors.leftMargin: Style.space(16)
        anchors.rightMargin: Style.space(16)

        PanelSectionHeader { text: "AUDIO BACKGROUND" }

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(10)
          Text {
            text: "Enabled"
            color: root.bar ? root.bar.foreground : "#fff"
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            Layout.fillWidth: true
          }
          ToggleSwitch {
            checked: root.running
            onToggled: root.setRunning(checked)
          }
        }

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(10)
          Text {
            text: "React to audio"
            color: root.bar ? root.bar.foreground : "#fff"
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            Layout.fillWidth: true
          }
          ToggleSwitch {
            checked: root.audio
            onToggled: root.setAudio(checked)
          }
        }

        PanelSectionHeader { text: "BACKGROUNDS" }

        // Enabled set with per-effect toggle; >1 enabled => rotates every 20s.
        Repeater {
          model: root.allEffects
          RowLayout {
            Layout.fillWidth: true
            spacing: Style.space(10)
            Button {
              text: modelData
              selected: root.effect === modelData
              onClicked: root.pickEffect(modelData)
              Layout.fillWidth: true
            }
            ToggleSwitch {
              checked: root.effects.indexOf(modelData) >= 0
              onToggled: root.toggleEffect(modelData, checked)
            }
          }
        }

        PanelSectionHeader { text: "INTENSITY" }
        PanelSlider {
          Layout.fillWidth: true
          bar: root.bar
          minimum: 0; maximum: 10; step: 1; integer: true
          value: root.intensity
          onMoved: function(v) { root.setIntensity(v) }
        }

        PanelSectionHeader { text: "INTRO BYLINE" }
        TextField {
          Layout.fillWidth: true
          text: root.byline
          placeholderText: "By x.com/@avillagran"
          onEditingFinished: root.setByline(text)
        }
      }
    }
  }
}
