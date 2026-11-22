import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// The server's bots, messenger style, with the server switcher at the bottom.
Rectangle {
    id: sidebar
    color: Theme.surface

    property var bots: []
    property var servers: []
    // Bots with a turn running; their chats can be left and come back to.
    readonly property var busyBots: JSON.parse(HermesApp.busyBots || "[]")
    signal manageServers()

    Rectangle {
        anchors.right: parent.right
        width: 1
        height: parent.height
        color: Theme.border
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.rightMargin: 1
        spacing: 0

        Label {
            text: "Bots"
            color: Theme.muted
            font.pixelSize: 12
            font.weight: Font.DemiBold
            font.capitalization: Font.AllUppercase
            Layout.leftMargin: 16
            Layout.topMargin: 18
            Layout.bottomMargin: 8
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: sidebar.bots
            spacing: 2
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

            // The selection glides between bots.
            currentIndex: sidebar.bots.findIndex(b => b.name === HermesApp.activeBot)
            highlightFollowsCurrentItem: true
            highlightMoveDuration: 220
            highlightMoveVelocity: -1
            highlightResizeDuration: 0
            highlight: Item {
                z: 0
                Rectangle {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    radius: Theme.radius
                    color: Theme.raised
                }
                Rectangle {
                    x: 8
                    width: 3
                    height: parent.height
                    color: Theme.accent
                }
            }

            delegate: Item {
                id: row
                required property var modelData
                readonly property bool active: modelData.name === HermesApp.activeBot
                readonly property bool busy: sidebar.busyBots.indexOf(modelData.name) >= 0
                width: ListView.view.width
                height: 60

                Rectangle {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    radius: Theme.radius
                    color: Qt.alpha(Theme.raised, hover.hovered && !row.active ? 0.5 : 0)
                    Behavior on color { ColorAnimation { duration: 140 } }
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 20
                    anchors.rightMargin: 16
                    spacing: 12

                    BotAvatar { name: row.modelData.displayName; size: 38 }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        Label {
                            Layout.fillWidth: true
                            text: row.modelData.displayName
                            color: Theme.foreground
                            font.pixelSize: 15
                            font.weight: row.active ? Font.DemiBold : Font.Medium
                            elide: Text.ElideRight
                        }
                        Label {
                            Layout.fillWidth: true
                            text: row.busy ? "Working…" : (row.modelData.preview || row.modelData.description || row.modelData.model)
                            visible: text.length > 0
                            color: Theme.muted
                            font.pixelSize: 13
                            elide: Text.ElideRight
                            maximumLineCount: 1
                        }
                    }

                    // Pulses while the bot is answering.
                    Rectangle {
                        visible: row.busy
                        width: 8
                        height: 8
                        radius: 4
                        color: Theme.accent
                        SequentialAnimation on opacity {
                            running: row.busy
                            loops: Animation.Infinite
                            NumberAnimation { from: 1; to: 0.3; duration: 700; easing.type: Easing.InOutSine }
                            NumberAnimation { from: 0.3; to: 1; duration: 700; easing.type: Easing.InOutSine }
                        }
                    }
                }

                HoverHandler { id: hover; cursorShape: row.active ? Qt.ArrowCursor : Qt.PointingHandCursor }
                TapHandler {
                    enabled: !row.active
                    onTapped: HermesApp.selectBot(row.modelData.name)
                }
            }

            Label {
                anchors.centerIn: parent
                width: parent.width - 32
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                visible: list.count === 0 && !HermesApp.chatLoading
                text: HermesApp.connected ? "No bots on this server" : ""
                color: Theme.muted
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.border }

        // Server switcher and connection state.
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 56

            Rectangle {
                anchors.fill: parent
                anchors.margins: 8
                radius: Theme.radius
                color: Qt.alpha(Theme.raised, serverHover.hovered ? 1 : 0)
                Behavior on color { ColorAnimation { duration: 140 } }
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 18
                anchors.rightMargin: 16
                spacing: 10

                Rectangle {
                    width: 8
                    height: 8
                    radius: 4
                    Behavior on color { ColorAnimation { duration: 300 } }
                    color: HermesApp.connected ? "#3fb950" : (HermesApp.status.indexOf("Connecting") === 0 || HermesApp.status.indexOf("Reconnecting") === 0 ? "#d29922" : Theme.danger)
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Label {
                        Layout.fillWidth: true
                        text: {
                            const s = sidebar.servers.find(s => s.id === HermesApp.activeServer)
                            return s ? s.name : "No server"
                        }
                        color: Theme.foreground
                        font.pixelSize: 13
                        font.weight: Font.Medium
                        elide: Text.ElideRight
                    }
                    Label {
                        Layout.fillWidth: true
                        text: HermesApp.connected ? "Connected" : HermesApp.status
                        color: Theme.muted
                        font.pixelSize: 12
                        elide: Text.ElideRight
                    }
                }
                Label { text: "⋯"; color: Theme.muted; font.pixelSize: 18 }
            }

            HoverHandler { id: serverHover; cursorShape: Qt.PointingHandCursor }
            TapHandler { onTapped: serverMenu.popup() }

            Menu {
                id: serverMenu
                // A Repeater inside a Menu leaks its items into the scene;
                // Instantiator adds them to the menu properly.
                Instantiator {
                    model: sidebar.servers
                    delegate: MenuItem {
                        required property var modelData
                        text: modelData.name
                        checkable: true
                        checked: modelData.id === HermesApp.activeServer
                        enabled: !HermesApp.streaming
                        onTriggered: if (modelData.id !== HermesApp.activeServer) HermesApp.selectServer(modelData.id)
                    }
                    onObjectAdded: (index, object) => serverMenu.insertItem(index, object)
                    onObjectRemoved: (index, object) => serverMenu.removeItem(object)
                }
                MenuSeparator { visible: sidebar.servers.length > 0 }
                MenuItem {
                    text: "Reconnect"
                    visible: !HermesApp.connected && sidebar.servers.length > 0
                    height: visible ? implicitHeight : 0
                    onTriggered: HermesApp.reconnect()
                }
                MenuItem { text: "Manage servers…"; onTriggered: sidebar.manageServers() }
            }
        }
    }
}
