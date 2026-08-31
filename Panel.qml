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
    "/.local/state/omarchy/audio-background/state.json"
  readonly property string writeState: Qt.resolvedUrl("bin/write_state.sh").toString().replace("file://", "")

  property bool running: true
  property bool audio: true
  property bool showFps: false
  property string effect: "matrix"
  property var effects: ["matrix", "rain", "wave", "bars", "donut", "fire", "starfield", "life"]
  property int intensity: 5
  property int introSize: 2
  property bool bootBetween: true
  property int rotateSecs: 20
  property string ttfxText: "OMARCHY"
  property int resolution: 1
  property int reactivity: 2
  property string byline: ""
  property string label: "♪"

  readonly property var allEffects: ["matrix", "rain", "wave", "bars", "donut", "fire", "starfield", "life"]
  // Vendored ttfx effects (rendered by the ttfx engine). matrix/rain stay hand-rolled
  // (ours are audio-reactive); these are the extra ttfx catalog worth offering.
  readonly property var ttfxEffects: ["beams", "blackhole", "bubbles", "burn", "colorshift", "fireworks", "rings", "synthgrid", "thunderstorm", "vhstape", "swarm", "spray"]

  // NOTE: opened / open / close / toggle / closeForPopoutSwitch are provided
  // by the qs.Ui.Panel base type — do NOT redeclare them (QML forbids
  // redefining base properties and the panel then fails to compile).
  function openFromHotkey() { root.open() }

  function refresh() { statusProc.running = true }

  function write(arg) { Quickshell.execDetached(["sh", root.writeState, arg]) }
  function setRunning(v)   { root.running = v;   write("running="   + (v ? "1" : "0")); }
  function setAudio(v)     { root.audio = v;     write("audio="     + (v ? "1" : "0")); }
  function setShowFps(v)   { root.showFps = v;   write("show_fps="  + (v ? "1" : "0")); }
  function setBootBetween(v) { root.bootBetween = v; write("boot_between=" + (v ? "1" : "0")); }
  function setRotateSecs(v) { root.rotateSecs = v;  write("rotate_secs=" + v); }
  function setResolution(v) { root.resolution = v;  write("resolution=" + v); }
  function setReactivity(v) { root.reactivity = v;  write("reactivity=" + v); }
  function setTtfxText(t)  { root.ttfxText = t;    write("ttfx_text=" + t); }
  function pickEffect(e)   { root.effect = e;    write("effect="    + e); }
  function setIntensity(v) { root.intensity = v; write("intensity=" + v); }
  function setIntroSize(v) {
    root.introSize = v
    write("intro_size=" + v)
    introReplay.restart()   // the intro renders once at startup; replay it at the new size (debounced)
  }
  function setByline(t)    { root.byline = t;    write("byline="    + t); }
  // Cycle the active effect across the full catalog (built-ins + ttfx) with ◀ ▶.
  function cycleEffect(dir) {
    var list = root.allEffects.concat(root.ttfxEffects)
    var i = list.indexOf(root.effect)
    if (i < 0) i = 0
    i = (i + dir + list.length) % list.length
    root.pickEffect(list[i])
  }
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

  // The intro (boot text) renders once at renderer startup, so a new intro_size only
  // shows on a restart. Debounce a restart until the size slider stops moving.
  Timer {
    id: introReplay
    interval: 700
    repeat: false
    onTriggered: root.write("restart")
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
          if (typeof s.audio === "boolean")   root.audio = s.audio
          if (typeof s.show_fps === "boolean") root.showFps = s.show_fps
          if (typeof s.effect === "string")   root.effect = s.effect
          if (Array.isArray(s.effects) && s.effects.length) root.effects = s.effects
          if (typeof s.intensity === "number") root.intensity = s.intensity
          if (typeof s.intro_size === "number") root.introSize = s.intro_size
          if (typeof s.boot_between === "boolean") root.bootBetween = s.boot_between
          if (typeof s.rotate_secs === "number") root.rotateSecs = s.rotate_secs
          if (typeof s.resolution === "number") root.resolution = s.resolution
          if (typeof s.reactivity === "number") root.reactivity = s.reactivity
          if (typeof s.ttfx_text === "string" && s.ttfx_text.trim() !== "") root.ttfxText = s.ttfx_text
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
    contentWidth: panel.fittedContentWidth(Style.space(700))
    // Height must follow the content or the card stays at the default
    // contentHeight (200) and the taller layout overflows below it, leaving
    // most controls rendered on bare desktop with no card behind them.
    contentHeight: panel.fittedContentHeight(contentColumn.implicitHeight)
    focusTarget: keyCatcher

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(d) { if (root.bar) root.bar.switchPanelFrom(root.barIdentity, d) }

      ColumnLayout {
        id: contentColumn
        width: parent.width
        spacing: Style.space(12)

        PanelSectionHeader { text: "AUDIO BACKGROUND" }

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(10)
          Text {
            text: "Enabled"
            color: root.barForeground
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            Layout.fillWidth: true
          }
          ToggleSwitch {
            checked: root.running
            // ToggleSwitch is a controlled component: toggled() passes no
            // value and it never mutates `checked` itself, so flip the real
            // state. Passing `checked` here would be a no-op.
            onToggled: root.setRunning(!root.running)
          }
        }

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(10)
          Text {
            text: "React to audio"
            color: root.barForeground
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            Layout.fillWidth: true
          }
          ToggleSwitch {
            checked: root.audio
            onToggled: root.setAudio(!root.audio)
          }
        }

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(10)
          Text {
            text: "Show FPS"
            color: root.barForeground
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            Layout.fillWidth: true
          }
          ToggleSwitch {
            checked: root.showFps
            onToggled: root.setShowFps(!root.showFps)
          }
        }

        PanelSectionHeader { text: "BACKGROUNDS" }

        // Prev / next effect across the whole catalog.
        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(8)
          Button { text: "◀"; onClicked: root.cycleEffect(-1) }
          Text {
            text: root.effect
            color: root.barForeground
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
            Layout.fillWidth: true
          }
          Button { text: "▶"; onClicked: root.cycleEffect(1) }
        }

        // Enabled set with per-effect toggle, 2 per row so the whole catalog fits
        // without making the panel too tall; >1 enabled => rotates (see ROTATION).
        GridLayout {
          columns: 2
          Layout.fillWidth: true
          columnSpacing: Style.space(10)
          rowSpacing: Style.space(6)
          Repeater {
            model: root.allEffects
            RowLayout {
              Layout.fillWidth: true
              spacing: Style.space(6)
              Button {
                text: modelData
                selected: root.effect === modelData
                onClicked: root.pickEffect(modelData)
                Layout.fillWidth: true
              }
              ToggleSwitch {
                checked: root.effects.indexOf(modelData) >= 0
                onToggled: root.toggleEffect(modelData, !(root.effects.indexOf(modelData) >= 0))
              }
            }
          }
        }

        PanelSectionHeader { text: "TTFX EFFECTS" }

        // Vendored ttfx effects; same pick + rotation-toggle as the built-ins.
        GridLayout {
          columns: 2
          Layout.fillWidth: true
          columnSpacing: Style.space(10)
          rowSpacing: Style.space(6)
          Repeater {
            model: root.ttfxEffects
            RowLayout {
              Layout.fillWidth: true
              spacing: Style.space(6)
              Button {
                text: modelData
                selected: root.effect === modelData
                onClicked: root.pickEffect(modelData)
                Layout.fillWidth: true
              }
              ToggleSwitch {
                checked: root.effects.indexOf(modelData) >= 0
                onToggled: root.toggleEffect(modelData, !(root.effects.indexOf(modelData) >= 0))
              }
            }
          }
        }

        PanelSectionHeader { text: "TTFX TEXT" }
        TextField {
          Layout.fillWidth: true
          // Default text the ttfx engine animates; emptying reverts to OMARCHY.
          text: root.ttfxText
          placeholderText: "OMARCHY"
          onEditingFinished: root.setTtfxText(text.trim() === "" ? "OMARCHY" : text)
        }

        PanelSectionHeader { text: "ROTATION" }

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(10)
          Text {
            text: "Boot between backgrounds"
            color: root.barForeground
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            Layout.fillWidth: true
          }
          ToggleSwitch {
            checked: root.bootBetween
            onToggled: root.setBootBetween(!root.bootBetween)
          }
        }

        PanelSectionHeader { text: "SECONDS PER BACKGROUND" }
        PanelSlider {
          Layout.fillWidth: true
          bar: root.bar
          minimum: 3; maximum: 120; step: 1; integer: true
          value: root.rotateSecs
          onMoved: function(v) { root.setRotateSecs(v) }
        }

        PanelSectionHeader { text: "INTENSITY" }
        PanelSlider {
          Layout.fillWidth: true
          bar: root.bar
          minimum: 0; maximum: 10; step: 1; integer: true
          value: root.intensity
          onMoved: function(v) { root.setIntensity(v) }
        }

        PanelSectionHeader { text: "RESOLUTION" }
        PanelSlider {
          Layout.fillWidth: true
          bar: root.bar
          // 1 = full-res (most CPU); higher = coarser cells = less CPU (old machines).
          minimum: 1; maximum: 4; step: 1; integer: true
          value: root.resolution
          onMoved: function(v) { root.setResolution(v) }
        }

        PanelSectionHeader { text: "AUDIO REACTIVITY" }
        PanelSlider {
          Layout.fillWidth: true
          bar: root.bar
          // How strongly ttfx effects speed up with the music. 0 = off, 2 = normal, 5 = strong.
          minimum: 0; maximum: 5; step: 1; integer: true
          value: root.reactivity
          onMoved: function(v) { root.setReactivity(v) }
        }

        PanelSectionHeader { text: "INTRO TEXT SIZE" }
        PanelSlider {
          Layout.fillWidth: true
          bar: root.bar
          minimum: 1; maximum: 3; step: 1; integer: true
          value: root.introSize
          onMoved: function(v) { root.setIntroSize(v) }
        }

        PanelSectionHeader { text: "INTRO BYLINE" }
        TextField {
          Layout.fillWidth: true
          // Default signature; emptying the field reverts to it.
          property string defaultByline: "By x.com/avillagran"
          text: root.byline.trim() === "" ? defaultByline : root.byline
          placeholderText: defaultByline
          onEditingFinished: {
            if (text.trim() === "") text = defaultByline
            // Empty means "use the built-in default" (Rust also defaults to it).
            root.setByline(text === defaultByline ? "" : text)
          }
        }
      }
    }
  }
}
