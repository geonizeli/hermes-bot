import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import HermesBot

ApplicationWindow {
    id: win
    width: 1280
    height: 860
    minimumWidth: 720
    minimumHeight: 520
    visible: true
    title: "Hermes Bot"
    color: Theme.background

    Material.theme: Material.Dark
    Material.accent: Theme.accent
    Material.background: Theme.background
    Material.foreground: Theme.foreground
    Material.roundedScale: Theme.radius

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // History sidebar; filled in by the sessions milestone.
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: 272
            color: Theme.surface

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 12
                spacing: 8

                Label {
                    text: "Hermes"
                    font.pixelSize: 18
                    font.weight: Font.DemiBold
                    color: Theme.foreground
                    Layout.leftMargin: 8
                    Layout.topMargin: 4
                }

                Item { Layout.fillHeight: true }
            }

            Rectangle {
                anchors.right: parent.right
                width: 1
                height: parent.height
                color: Theme.border
            }
        }

        // Conversation area: an empty state with the composer centered, the
        // way Grok greets a new chat.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ColumnLayout {
                anchors.centerIn: parent
                width: Math.min(parent.width - 64, 760)
                spacing: 28

                Label {
                    Layout.alignment: Qt.AlignHCenter
                    text: "What do you want to know?"
                    font.pixelSize: 30
                    color: Theme.foreground
                }

                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 116
                    radius: Math.max(Theme.radius * 2, 24)
                    color: Theme.raised
                    border.color: input.activeFocus ? Theme.accent : Theme.border
                    border.width: 1

                    TextArea {
                        id: input
                        anchors.fill: parent
                        anchors.margins: 14
                        anchors.bottomMargin: 52
                        placeholderText: "Ask anything"
                        placeholderTextColor: Theme.muted
                        color: Theme.foreground
                        wrapMode: TextEdit.Wrap
                        font.pixelSize: 16
                        background: null
                        focus: true
                    }

                    RoundButton {
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        anchors.margins: 10
                        width: 40
                        height: 40
                        enabled: input.text.trim().length > 0
                        text: "↑"
                        font.pixelSize: 18
                        Material.background: enabled ? Theme.accent : Theme.border
                        Material.foreground: Theme.accentForeground
                    }
                }
            }
        }
    }
}
