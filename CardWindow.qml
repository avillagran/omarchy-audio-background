import QtQuick
import Quickshell
import Quickshell.Wayland
import qs.Commons
import qs.Ui

// Transparent card host — a drop-in replacement for Omarchy's KeyboardPanel that does
// NOT paint an opaque popup background (KeyboardPanel's internal BorderSurface is
// hardcoded to Color.popups.background, which makes real transparency impossible).
// This window is fully transparent; the plugin draws its own translucent card, so the
// animated background shows through. Anchoring math mirrors Ui/KeyboardPanel.qml.
// (Same pattern as omarchy-supernotch's CardWindow.)
PanelWindow {
  id: root

  required property Item anchorItem
  required property var bar
  property var owner: null
  property int contentWidth: Style.space(280)
  property int contentHeight: Style.space(200)
  property bool open: false
  property bool centerOnBar: true
  property int gap: Style.gapsOut
  property int margin: Style.gapsOut
  property int padding: Style.space(12)
  property Item focusTarget: null

  // REAL transparency: the window itself is transparent.
  color: "transparent"
  screen: anchorWindow ? anchorWindow.screen : null
  visible: root.open
  anchors.top: true; anchors.left: true; anchors.right: true; anchors.bottom: true
  mask: Region { width: root.screenW; height: root.screenH }

  // Grab keyboard focus while open so PanelKeyCatcher's Esc/keys reach us.
  WlrLayershell.namespace: "omarchy-audio-background"
  WlrLayershell.layer: WlrLayer.Overlay
  WlrLayershell.keyboardFocus: open ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

  readonly property var anchorWindow: anchorItem ? anchorItem.QsWindow.window : null
  readonly property string barPos: bar ? bar.position : "top"
  readonly property real barW: anchorWindow ? anchorWindow.width : (screen ? screen.width : 0)
  readonly property real barH: anchorWindow ? anchorWindow.height : 0
  readonly property real screenW: screen ? screen.width : 0
  readonly property real screenH: screen ? screen.height : 0

  // Track layout changes between the bar's contentItem and the anchor item, so the
  // position binding stays reactive (mapToItem on its own is a one-shot).
  TransformWatcher {
    id: anchorWatcher
    a: anchorWindow ? anchorWindow.contentItem : null
    b: anchorItem
  }
  readonly property point anchorScreenPos: {
    anchorWatcher.transform  // reactive dependency
    if (!anchorItem || !anchorWindow) return Qt.point(0, 0)
    return anchorItem.mapToItem(anchorWindow.contentItem, 0, 0)
  }
  readonly property real anchorW: anchorItem ? anchorItem.width : 0
  readonly property real anchorH: anchorItem ? anchorItem.height : 0

  // Desired top-left of the card in screen coordinates (mirrors KeyboardPanel).
  readonly property point cardOrigin: {
    if (!anchorItem || !bar) return Qt.point(margin, margin)
    var x = 0, y = 0
    if (centerOnBar && (barPos === "top" || barPos === "bottom")) {
      x = screenW / 2 - contentWidth / 2
      y = barPos === "bottom" ? screenH - barH - contentHeight - gap : barH + gap
    } else if (centerOnBar) {
      x = barPos === "left" ? barW + gap : screenW - barW - contentWidth - gap
      y = screenH / 2 - contentHeight / 2
    } else if (barPos === "bottom") {
      x = anchorScreenPos.x + anchorW / 2 - contentWidth / 2
      y = screenH - barH - contentHeight - gap
    } else if (barPos === "left") {
      x = barW + gap
      y = anchorScreenPos.y + anchorH / 2 - contentHeight / 2
    } else if (barPos === "right") {
      x = screenW - barW - contentWidth - gap
      y = anchorScreenPos.y + anchorH / 2 - contentHeight / 2
    } else {
      x = anchorScreenPos.x
      y = barH + gap
    }
    x = Math.max(margin, Math.min(x, screenW - contentWidth - margin))
    y = Math.max(margin, Math.min(y, screenH - contentHeight - margin))
    return Qt.point(Math.round(x), Math.round(y))
  }

  // Size-to-content helpers (from KeyboardPanel) so the panel caps at the screen.
  readonly property real availableCardWidth: screenW > 0
    ? Math.max(120, screenW - ((barPos === "left" || barPos === "right") ? barW + gap + margin : margin * 2)) : 0
  readonly property real availableCardHeight: screenH > 0
    ? Math.max(120, screenH - ((barPos === "top" || barPos === "bottom") ? barH + gap + margin : margin * 2)) : 0
  readonly property real verticalContentInset: padding * 2
  function fittedContentWidth(width, cap) {
    var desired = Math.max(1, Number(width) || 1)
    var maxWidth = root.availableCardWidth > 0 ? root.availableCardWidth : desired
    if (cap !== undefined && Number(cap) > 0) maxWidth = Math.min(maxWidth, Number(cap))
    return Math.round(Math.min(desired, maxWidth))
  }
  function fittedContentHeight(implicitHeight, cap) {
    var desired = Math.max(root.verticalContentInset, (Number(implicitHeight) || 0) + root.verticalContentInset)
    var maxHeight = root.availableCardHeight > 0 ? root.availableCardHeight : desired
    if (cap !== undefined && Number(cap) > 0) maxHeight = Math.min(maxHeight, Number(cap))
    return Math.round(Math.min(desired, maxHeight))
  }

  function close() { if (owner && "close" in owner) owner.close(); else root.open = false }

  onOpenChanged: {
    if (root.open && root.focusTarget) Qt.callLater(function () {
      if (root.open && root.focusTarget) root.focusTarget.forceActiveFocus()
    })
  }

  // Outside-click dismissal.
  MouseArea { anchors.fill: parent; onClicked: root.close() }

  // Plugin content lands here (positioned at cardOrigin).
  default property alias contentItem: cardHolder.children
  Item {
    id: cardHolder
    x: root.cardOrigin.x
    y: root.cardOrigin.y
    width: root.contentWidth
    height: root.contentHeight
    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons }
  }
}
