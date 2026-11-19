import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

ApplicationWindow {
    id: win
    width: 1200
    height: 820
    minimumWidth: 720
    minimumHeight: 480
    visible: true
    title: "Hermes Bot"

    readonly property var servers: JSON.parse(HermesApp.servers || "[]")
    readonly property var bots: JSON.parse(HermesApp.bots || "[]")
    readonly property var activeBotInfo: bots.find(b => b.name === HermesApp.activeBot) || null
    readonly property string botName: activeBotInfo ? activeBotInfo.displayName : "the bot"

    ListModel { id: messages }

    // The open turn's assistant row, created on the first streamed text.
    property int assistantRow: -1

    function ensureAssistantRow() {
        if (assistantRow < 0) {
            messages.append({ role: "assistant", text: "", reasoning: "" })
            assistantRow = messages.count - 1
        }
    }

    Connections {
        target: HermesApp
        function onTranscriptLoaded(json) {
            messages.clear()
            win.assistantRow = -1
            for (const row of JSON.parse(json))
                messages.append(row)
            conversation.positionViewAtEnd()
        }
        function onUserMessage(text) {
            messages.append({ role: "user", text: text, reasoning: "" })
            win.assistantRow = -1
            conversation.positionViewAtEnd()
        }
        function onAssistantStarted() { win.ensureAssistantRow() }
        function onAssistantDelta(text) {
            win.ensureAssistantRow()
            messages.setProperty(win.assistantRow, "text", messages.get(win.assistantRow).text + text)
            conversation.positionViewAtEnd()
        }
        function onReasoningDelta(text) {
            win.ensureAssistantRow()
            messages.setProperty(win.assistantRow, "reasoning", messages.get(win.assistantRow).reasoning + text)
        }
        function onToolStarted(name, detail) {
            messages.append({ role: "tool", text: detail ? name + " — " + detail : name, reasoning: "" })
            // Text after a tool call starts a new assistant row below it.
            win.assistantRow = -1
            conversation.positionViewAtEnd()
        }
        function onTurnFinished(status, text, error) {
            if (text.length > 0) {
                win.ensureAssistantRow()
                messages.setProperty(win.assistantRow, "text", text)
            }
            if (status === "error")
                messages.append({ role: "error", text: error || "The bot reported an error.", reasoning: "" })
            else if (status === "interrupted")
                messages.append({ role: "info", text: "Stopped.", reasoning: "" })
            win.assistantRow = -1
            conversation.positionViewAtEnd()
        }
        function onServerRequest(id, method, params) {
            requestDialog.open_(id, method, JSON.parse(params))
        }
    }

    Component.onCompleted: if (servers.length === 0) setupDialog.open()

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            spacing: 8

            Label { text: "Server" }
            ComboBox {
                id: serverBox
                Layout.preferredWidth: 220
                model: win.servers
                textRole: "name"
                valueRole: "id"
                enabled: win.servers.length > 0 && !HermesApp.streaming
                currentIndex: win.servers.findIndex(s => s.id === HermesApp.activeServer)
                onActivated: HermesApp.selectServer(currentValue)
            }
            Button { text: "Servers…"; onClicked: setupDialog.open() }

            ToolSeparator {}

            Button {
                text: "New chat"
                enabled: HermesApp.connected && !HermesApp.streaming
                onClicked: HermesApp.newChat()
            }

            Item { Layout.fillWidth: true }

            Label {
                text: HermesApp.status
                elide: Text.ElideRight
                Layout.maximumWidth: 320
            }
            Button {
                text: "Reconnect"
                visible: !HermesApp.connected && win.servers.length > 0
                onClicked: HermesApp.reconnect()
            }
        }
    }

    SplitView {
        anchors.fill: parent

        // The server's bots; selecting one opens its chat.
        Pane {
            SplitView.preferredWidth: 240
            SplitView.minimumWidth: 160
            padding: 0

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                Label {
                    text: "Bots"
                    font.bold: true
                    padding: 8
                }
                ListView {
                    id: botList
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: win.bots
                    ScrollBar.vertical: ScrollBar {}
                    delegate: ItemDelegate {
                        required property var modelData
                        width: ListView.view.width
                        highlighted: modelData.name === HermesApp.activeBot
                        enabled: !HermesApp.streaming || highlighted
                        onClicked: if (!highlighted) HermesApp.selectBot(modelData.name)
                        contentItem: ColumnLayout {
                            spacing: 2
                            Label {
                                Layout.fillWidth: true
                                text: modelData.displayName
                                font.bold: true
                                elide: Text.ElideRight
                            }
                            Label {
                                Layout.fillWidth: true
                                text: modelData.description || modelData.model
                                visible: text.length > 0
                                elide: Text.ElideRight
                                opacity: 0.7
                            }
                        }
                    }
                    Label {
                        anchors.centerIn: parent
                        visible: botList.count === 0
                        text: HermesApp.connected ? "No bots on this server" : ""
                        opacity: 0.6
                    }
                }
            }
        }

        // The conversation and the composer.
        Pane {
            SplitView.fillWidth: true
            padding: 0

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                ListView {
                    id: conversation
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 10
                    topMargin: 12
                    bottomMargin: 12
                    model: messages
                    ScrollBar.vertical: ScrollBar {}

                    delegate: Item {
                        id: row
                        required property string role
                        required property string text
                        required property string reasoning
                        width: ListView.view.width
                        height: bubble.height

                        Frame {
                            id: bubble
                            x: row.role === "user" ? row.width - width - 16 : 16
                            width: row.role === "user"
                                   ? Math.min(implicitWidth, row.width * 0.75)
                                   : row.width - 32
                            padding: row.role === "tool" || row.role === "info" ? 4 : 10
                            opacity: row.role === "tool" || row.role === "info" ? 0.7 : 1

                            ColumnLayout {
                                width: parent.width
                                spacing: 6

                                Label {
                                    text: row.role === "user" ? "You"
                                          : row.role === "assistant" ? win.botName
                                          : row.role === "tool" ? "Tool"
                                          : row.role === "error" ? "Error" : ""
                                    visible: text.length > 0
                                    font.bold: true
                                    color: row.role === "error" ? "red" : palette.text
                                }
                                Label {
                                    visible: row.reasoning.length > 0
                                    Layout.fillWidth: true
                                    text: row.reasoning
                                    wrapMode: Text.Wrap
                                    font.italic: true
                                    opacity: 0.6
                                    maximumLineCount: 6
                                    elide: Text.ElideRight
                                }
                                TextEdit {
                                    Layout.fillWidth: true
                                    text: row.text.length > 0 ? row.text : (row.role === "assistant" ? "…" : "")
                                    textFormat: row.role === "assistant" ? TextEdit.MarkdownText : TextEdit.PlainText
                                    wrapMode: TextEdit.Wrap
                                    readOnly: true
                                    selectByMouse: true
                                    color: palette.text
                                    selectionColor: palette.highlight
                                    selectedTextColor: palette.highlightedText
                                    onLinkActivated: link => Qt.openUrlExternally(link)
                                }
                            }
                        }
                    }

                    Label {
                        anchors.centerIn: parent
                        visible: conversation.count === 0
                        text: HermesApp.connected ? "Start chatting with " + win.botName
                                                  : (win.servers.length === 0 ? "Add a Hermes server to begin." : "")
                        opacity: 0.6
                    }
                }

                // Errors stay visible until the next action clears them.
                Frame {
                    Layout.fillWidth: true
                    visible: HermesApp.error.length > 0
                    RowLayout {
                        anchors.fill: parent
                        Label {
                            Layout.fillWidth: true
                            text: HermesApp.error
                            color: "red"
                            wrapMode: Text.Wrap
                        }
                        Button { text: "Dismiss"; flat: true; onClicked: HermesApp.error = "" }
                    }
                }

                Frame {
                    Layout.fillWidth: true

                    RowLayout {
                        anchors.fill: parent
                        spacing: 8

                        ScrollView {
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.min(Math.max(input.implicitHeight, 40), 200)

                            TextArea {
                                id: input
                                placeholderText: HermesApp.connected ? "Message " + win.botName + "  (Enter to send, Shift+Enter for a new line)" : "Not connected"
                                wrapMode: TextEdit.Wrap
                                enabled: HermesApp.connected
                                focus: true
                                Keys.onReturnPressed: event => {
                                    if (event.modifiers & Qt.ShiftModifier) {
                                        event.accepted = false
                                    } else {
                                        send()
                                    }
                                }
                                Keys.onEnterPressed: event => Keys.returnPressed(event)
                                function send() {
                                    if (HermesApp.streaming || text.trim().length === 0)
                                        return
                                    HermesApp.send(text)
                                    text = ""
                                }
                            }
                        }
                        Button {
                            text: HermesApp.streaming ? "Stop" : "Send"
                            enabled: HermesApp.connected && (HermesApp.streaming || input.text.trim().length > 0)
                            onClicked: HermesApp.streaming ? HermesApp.stop() : input.send()
                        }
                    }
                }
            }
        }
    }

    SetupDialog {
        id: setupDialog
        anchors.centerIn: parent
        servers: win.servers
    }

    RequestDialog {
        id: requestDialog
        anchors.centerIn: parent
    }
}
