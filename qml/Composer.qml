import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// The message box: grows with its text, Enter sends, Shift+Enter breaks the
// line, and the round button turns into Stop while the bot answers.
Rectangle {
    id: composer
    property string botName: "the bot"

    implicitHeight: Math.min(Math.max(input.implicitHeight, 24), 220) + 28
    radius: Math.max(Theme.radius, 0)
    color: Theme.raised
    border.width: 1
    border.color: input.activeFocus ? Qt.alpha(Theme.accent, 0.6) : Theme.border
    Behavior on border.color { ColorAnimation { duration: 160 } }
    Behavior on implicitHeight { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

    function focusInput() { input.forceActiveFocus() }

    function send() {
        if (HermesApp.streaming || input.text.trim().length === 0)
            return
        HermesApp.send(input.text)
        input.text = ""
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 16
        anchors.rightMargin: 10
        anchors.topMargin: 10
        anchors.bottomMargin: 10
        spacing: 10

        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.alignment: Qt.AlignVCenter

            TextArea {
                id: input
                placeholderText: HermesApp.connected ? "Message " + composer.botName : "Not connected"
                placeholderTextColor: Theme.muted
                color: Theme.foreground
                selectionColor: Qt.alpha(Theme.accent, 0.4)
                selectedTextColor: Theme.foreground
                font.pixelSize: 15
                wrapMode: TextEdit.Wrap
                enabled: HermesApp.connected
                focus: true
                background: null
                padding: 4
                verticalAlignment: TextEdit.AlignVCenter
                Keys.onReturnPressed: event => {
                    if (event.modifiers & Qt.ShiftModifier)
                        event.accepted = false
                    else
                        composer.send()
                }
                Keys.onEnterPressed: event => Keys.returnPressed(event)
            }
        }

        // Send / Stop.
        Rectangle {
            id: action
            readonly property bool canSend: HermesApp.connected && input.text.trim().length > 0
            readonly property bool live: HermesApp.streaming || canSend
            Layout.alignment: Qt.AlignBottom
            width: 36
            height: 36
            radius: 18
            color: live ? Theme.accent : Theme.border
            opacity: actionHover.hovered && live ? 0.85 : 1
            scale: actionTap.pressed && live ? 0.9 : 1
            Behavior on color { ColorAnimation { duration: 160 } }
            Behavior on opacity { NumberAnimation { duration: 120 } }
            Behavior on scale { NumberAnimation { duration: 100; easing.type: Easing.OutCubic } }

            // Arrow up to send, a square to stop.
            Canvas {
                anchors.centerIn: parent
                width: 16
                height: 16
                visible: !HermesApp.streaming
                property color ink: action.live ? Theme.accentForeground : Theme.muted
                onInkChanged: requestPaint()
                onPaint: {
                    const ctx = getContext("2d")
                    ctx.reset()
                    ctx.strokeStyle = ink
                    ctx.lineWidth = 2
                    ctx.lineCap = "round"
                    ctx.lineJoin = "round"
                    ctx.beginPath()
                    ctx.moveTo(8, 14); ctx.lineTo(8, 2)
                    ctx.moveTo(3, 7); ctx.lineTo(8, 2); ctx.lineTo(13, 7)
                    ctx.stroke()
                }
            }
            Rectangle {
                anchors.centerIn: parent
                visible: HermesApp.streaming
                width: 12
                height: 12
                radius: 2
                color: Theme.accentForeground
            }

            HoverHandler { id: actionHover; cursorShape: action.live ? Qt.PointingHandCursor : Qt.ArrowCursor }
            TapHandler {
                id: actionTap
                enabled: action.live
                onTapped: HermesApp.streaming ? HermesApp.stop() : composer.send()
            }
        }
    }
}
