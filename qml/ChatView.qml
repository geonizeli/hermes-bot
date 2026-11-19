import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Qt.labs.qmlmodels
import HermesBot

// The open Bot Chat: a header, the conversation in a centered column, and
// the composer, which sits in the middle while the chat is empty.
Item {
    id: view
    property var bot: null
    readonly property string botName: bot ? bot.displayName : "the bot"
    readonly property bool empty: messages.count === 0
    readonly property int columnWidth: Math.min(width - 48, 780)

    // Every row carries every role so the ListModel keeps one schema.
    ListModel { id: messages }

    property int assistantRow: -1
    property var thinkingSince: 0

    function addRow(fields) {
        messages.append(Object.assign({
            role: "assistant", text: "", reasoning: "", toolId: "", name: "",
            detail: "", summary: "", done: true, thoughtMs: 0
        }, fields))
        return messages.count - 1
    }

    function ensureAssistantRow() {
        if (assistantRow < 0)
            assistantRow = addRow({ role: "assistant", done: false })
    }

    function closeThinking() {
        if (assistantRow >= 0 && thinkingSince > 0) {
            messages.setProperty(assistantRow, "thoughtMs", Date.now() - thinkingSince)
            thinkingSince = 0
        }
    }

    // Stick to the bottom while the user is there; Markdown rows settle their
    // height after layout, so follow content growth rather than scroll once.
    property bool follow: true
    function scrollToEnd() {
        follow = true
        Qt.callLater(conversation.positionViewAtEnd)
    }

    Connections {
        target: HermesApp
        function onTranscriptLoaded(json) {
            messages.clear()
            view.assistantRow = -1
            view.thinkingSince = 0
            for (const row of JSON.parse(json)) {
                if (row.role === "tool")
                    view.addRow({ role: "tool", name: row.text })
                else
                    view.addRow({ role: row.role, text: row.text, reasoning: row.reasoning || "" })
            }
            view.scrollToEnd()
            composer.focusInput()
        }
        function onUserMessage(text) {
            view.addRow({ role: "user", text: text })
            view.assistantRow = -1
            view.scrollToEnd()
        }
        function onAssistantStarted() { view.ensureAssistantRow() }
        function onAssistantDelta(text) {
            view.ensureAssistantRow()
            view.closeThinking()
            messages.setProperty(view.assistantRow, "text", messages.get(view.assistantRow).text + text)
            view.scrollToEnd()
        }
        function onReasoningDelta(text) {
            view.ensureAssistantRow()
            if (view.thinkingSince === 0 && messages.get(view.assistantRow).text.length === 0)
                view.thinkingSince = Date.now()
            messages.setProperty(view.assistantRow, "reasoning", messages.get(view.assistantRow).reasoning + text)
            view.scrollToEnd()
        }
        function onToolStarted(id, name, detail) {
            view.closeThinking()
            if (view.assistantRow >= 0)
                messages.setProperty(view.assistantRow, "done", true)
            view.addRow({ role: "tool", toolId: id, name: name, detail: detail, done: false })
            // Text after a tool call starts a new answer below it.
            view.assistantRow = -1
            view.scrollToEnd()
        }
        function onToolFinished(id, name, summary) {
            for (let i = messages.count - 1; i >= 0; i--) {
                const row = messages.get(i)
                if (row.role === "tool" && !row.done && (row.toolId === id || (!id && row.name === name))) {
                    messages.setProperty(i, "summary", summary)
                    messages.setProperty(i, "done", true)
                    break
                }
            }
        }
        function onTurnFinished(status, text, error) {
            view.closeThinking()
            if (text.length > 0) {
                view.ensureAssistantRow()
                messages.setProperty(view.assistantRow, "text", text)
            }
            if (view.assistantRow >= 0)
                messages.setProperty(view.assistantRow, "done", true)
            for (let i = 0; i < messages.count; i++)
                if (messages.get(i).role === "tool" && !messages.get(i).done)
                    messages.setProperty(i, "done", true)
            if (status === "error")
                view.addRow({ role: "error", text: error || "The bot reported an error." })
            else if (status === "interrupted")
                view.addRow({ role: "info", text: "Stopped" })
            view.assistantRow = -1
            view.scrollToEnd()
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Header: who you're talking to.
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 56
            visible: view.bot !== null && !view.empty

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 20
                anchors.rightMargin: 20
                spacing: 10
                BotAvatar { name: view.botName; size: 28 }
                Label {
                    text: view.botName
                    color: Theme.foreground
                    font.pixelSize: 15
                    font.weight: Font.DemiBold
                }
                Label {
                    Layout.fillWidth: true
                    text: view.bot ? view.bot.model : ""
                    color: Theme.muted
                    font.pixelSize: 13
                    elide: Text.ElideRight
                }
            }
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
        }

        Item { Layout.fillHeight: true; visible: view.empty }

        // Empty chat: introduce the bot above the centered composer.
        ColumnLayout {
            visible: view.empty
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: view.columnWidth
            Layout.bottomMargin: 28
            spacing: 10

            BotAvatar {
                Layout.alignment: Qt.AlignHCenter
                name: view.botName
                size: 64
                visible: view.bot !== null
            }
            Label {
                Layout.alignment: Qt.AlignHCenter
                text: view.bot ? view.botName : (HermesApp.connected ? "Pick a bot" : "Not connected")
                color: Theme.foreground
                font.pixelSize: 28
                font.weight: Font.DemiBold
            }
            Label {
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumWidth: view.columnWidth
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                text: view.bot ? (view.bot.description || "Say hello to start your chat.") : HermesApp.status
                color: Theme.muted
                font.pixelSize: 15
            }
        }

        ListView {
            id: conversation
            visible: !view.empty
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 18
            topMargin: 24
            bottomMargin: 24
            model: messages
            boundsBehavior: Flickable.StopAtBounds
            onContentHeightChanged: if (view.follow) Qt.callLater(positionViewAtEnd)
            onHeightChanged: if (view.follow) Qt.callLater(positionViewAtEnd)
            onMovementEnded: view.follow = atYEnd
            onFlickEnded: view.follow = atYEnd
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

            delegate: DelegateChooser {
                role: "role"

                DelegateChoice {
                    roleValue: "user"
                    Item {
                        id: userRow
                        required property string text
                        width: ListView.view.width
                        height: bubble.height
                        Rectangle {
                            id: bubble
                            readonly property real maxWidth: view.columnWidth * 0.8
                            x: (userRow.width + view.columnWidth) / 2 - width
                            width: Math.min(userText.implicitWidth + 32, maxWidth)
                            height: userText.height + 22
                            radius: Theme.radius
                            color: Theme.raised
                            TextEdit {
                                id: userText
                                x: 16
                                y: 11
                                width: Math.min(implicitWidth, bubble.maxWidth - 32)
                                text: userRow.text
                                color: Theme.foreground
                                font.pixelSize: 15
                                wrapMode: TextEdit.Wrap
                                readOnly: true
                                selectByMouse: true
                                selectionColor: Qt.alpha(Theme.accent, 0.4)
                            }
                        }
                    }
                }

                DelegateChoice {
                    roleValue: "assistant"
                    Item {
                        id: botRow
                        required property int index
                        required property string text
                        required property string reasoning
                        required property bool done
                        required property int thoughtMs
                        property bool showReasoning: false
                        width: ListView.view.width
                        height: column.height

                        ColumnLayout {
                            id: column
                            x: (botRow.width - view.columnWidth) / 2
                            width: view.columnWidth
                            spacing: 8

                            // "Thinking…" while it reasons, then a toggle for the reasoning.
                            Item {
                                visible: botRow.reasoning.length > 0
                                Layout.preferredHeight: 22
                                Layout.preferredWidth: thinkLabel.implicitWidth + 4
                                Label {
                                    id: thinkLabel
                                    anchors.verticalCenter: parent.verticalCenter
                                    readonly property bool thinking: !botRow.done && botRow.text.length === 0
                                    text: thinking ? "Thinking…"
                                          : (botRow.thoughtMs > 0 ? "Thought for " + Math.max(1, Math.round(botRow.thoughtMs / 1000)) + "s" : "Reasoning")
                                          + (botRow.showReasoning ? "  ▾" : "  ▸")
                                    color: Theme.muted
                                    font.pixelSize: 13
                                    SequentialAnimation on opacity {
                                        running: thinkLabel.thinking
                                        loops: Animation.Infinite
                                        alwaysRunToEnd: true
                                        NumberAnimation { to: 0.4; duration: 700 }
                                        NumberAnimation { to: 1; duration: 700 }
                                    }
                                }
                                HoverHandler { cursorShape: Qt.PointingHandCursor }
                                TapHandler { onTapped: botRow.showReasoning = !botRow.showReasoning }
                            }
                            Rectangle {
                                visible: botRow.showReasoning && botRow.reasoning.length > 0
                                Layout.fillWidth: true
                                Layout.preferredHeight: reasoningText.height
                                color: "transparent"
                                Rectangle { width: 2; height: parent.height; color: Theme.border }
                                Label {
                                    id: reasoningText
                                    x: 14
                                    width: parent.width - 14
                                    text: botRow.reasoning
                                    color: Theme.muted
                                    font.pixelSize: 14
                                    wrapMode: Text.Wrap
                                }
                            }

                            TextEdit {
                                Layout.fillWidth: true
                                visible: botRow.text.length > 0
                                text: botRow.text
                                textFormat: TextEdit.MarkdownText
                                color: Theme.foreground
                                font.pixelSize: 15
                                wrapMode: TextEdit.Wrap
                                readOnly: true
                                selectByMouse: true
                                selectionColor: Qt.alpha(Theme.accent, 0.4)
                                onLinkActivated: link => Qt.openUrlExternally(link)
                                HoverHandler {
                                    cursorShape: parent.hoveredLink ? Qt.PointingHandCursor : Qt.IBeamCursor
                                }
                            }

                            // Copy, once the answer is complete.
                            Label {
                                id: copyLabel
                                visible: botRow.done && botRow.text.length > 0
                                text: copied ? "Copied" : "Copy"
                                property bool copied: false
                                color: copyHover.hovered ? Theme.foreground : Theme.muted
                                font.pixelSize: 12
                                opacity: copyHover.hovered || rowHover.hovered ? 1 : 0
                                HoverHandler { id: copyHover; cursorShape: Qt.PointingHandCursor }
                                TapHandler {
                                    onTapped: {
                                        clipboard.text = botRow.text
                                        clipboard.selectAll()
                                        clipboard.copy()
                                        copyLabel.copied = true
                                        copyReset.restart()
                                    }
                                }
                                Timer { id: copyReset; interval: 1500; onTriggered: copyLabel.copied = false }
                            }
                        }
                        HoverHandler { id: rowHover }
                    }
                }

                DelegateChoice {
                    roleValue: "tool"
                    Item {
                        id: toolRow
                        required property string name
                        required property string detail
                        required property string summary
                        required property bool done
                        property bool expanded: false
                        width: ListView.view.width
                        height: toolColumn.height

                        ColumnLayout {
                            id: toolColumn
                            x: (toolRow.width - view.columnWidth) / 2
                            width: view.columnWidth
                            spacing: 6

                            Rectangle {
                                Layout.preferredHeight: 28
                                Layout.preferredWidth: chipRow.implicitWidth + 20
                                radius: Theme.radius
                                color: chipHover.hovered ? Theme.raised : "transparent"
                                border.width: 1
                                border.color: Theme.border
                                RowLayout {
                                    id: chipRow
                                    anchors.centerIn: parent
                                    spacing: 8
                                    BusyIndicator {
                                        visible: !toolRow.done
                                        running: visible
                                        Layout.preferredWidth: 14
                                        Layout.preferredHeight: 14
                                    }
                                    Label {
                                        visible: toolRow.done
                                        text: "✓"
                                        color: Theme.muted
                                        font.pixelSize: 12
                                    }
                                    Label {
                                        text: toolRow.name || "tool"
                                        color: Theme.muted
                                        font.pixelSize: 13
                                        font.family: "monospace"
                                    }
                                    Label {
                                        visible: toolRow.detail.length > 0 || toolRow.summary.length > 0
                                        text: toolRow.expanded ? "▾" : "▸"
                                        color: Theme.muted
                                        font.pixelSize: 11
                                    }
                                }
                                HoverHandler { id: chipHover; cursorShape: Qt.PointingHandCursor }
                                TapHandler { onTapped: toolRow.expanded = !toolRow.expanded }
                            }
                            Label {
                                visible: toolRow.expanded && (toolRow.detail.length > 0 || toolRow.summary.length > 0)
                                Layout.fillWidth: true
                                text: [toolRow.detail, toolRow.summary].filter(s => s.length > 0).join("\n\n")
                                color: Theme.muted
                                font.pixelSize: 12
                                font.family: "monospace"
                                wrapMode: Text.WrapAnywhere
                                maximumLineCount: 30
                                elide: Text.ElideRight
                                leftPadding: 12
                                Rectangle { width: 2; height: parent.height; color: Theme.border }
                            }
                        }
                    }
                }

                DelegateChoice {
                    roleValue: "error"
                    Item {
                        id: errorRow
                        required property string text
                        width: ListView.view.width
                        height: errorLabel.height
                        Label {
                            id: errorLabel
                            x: (errorRow.width - view.columnWidth) / 2
                            width: view.columnWidth
                            text: errorRow.text
                            color: Theme.danger
                            font.pixelSize: 14
                            wrapMode: Text.Wrap
                        }
                    }
                }

                DelegateChoice {
                    roleValue: "info"
                    Item {
                        id: infoRow
                        required property string text
                        width: ListView.view.width
                        height: infoLabel.height
                        Label {
                            id: infoLabel
                            x: (infoRow.width - view.columnWidth) / 2
                            width: view.columnWidth
                            text: infoRow.text
                            color: Theme.muted
                            font.pixelSize: 13
                            font.italic: true
                        }
                    }
                }
            }

            // Waiting for the first word of an answer.
            footer: Item {
                width: ListView.view ? ListView.view.width : 0
                height: waiting.visible ? 36 : 0
                Row {
                    id: waiting
                    visible: HermesApp.streaming && view.assistantRow < 0
                             && messages.count > 0 && messages.get(messages.count - 1).role === "user"
                    x: (parent.width - view.columnWidth) / 2
                    y: 14
                    spacing: 6
                    Repeater {
                        model: 3
                        Rectangle {
                            required property int index
                            width: 7
                            height: 7
                            radius: 3.5
                            color: Theme.muted
                            SequentialAnimation on opacity {
                                running: waiting.visible
                                loops: Animation.Infinite
                                PauseAnimation { duration: index * 160 }
                                NumberAnimation { to: 0.25; duration: 400 }
                                NumberAnimation { to: 1; duration: 400 }
                                PauseAnimation { duration: (2 - index) * 160 }
                            }
                        }
                    }
                }
            }
        }

        // Errors stay until dismissed or the next action clears them.
        Rectangle {
            visible: HermesApp.error.length > 0
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: view.columnWidth
            Layout.preferredHeight: errorText.implicitHeight + 20
            Layout.bottomMargin: 8
            radius: Theme.radius
            color: Qt.alpha(Theme.danger, 0.12)
            border.width: 1
            border.color: Qt.alpha(Theme.danger, 0.5)
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 14
                anchors.rightMargin: 10
                Label {
                    id: errorText
                    Layout.fillWidth: true
                    text: HermesApp.error
                    color: Theme.foreground
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                }
                Label {
                    text: "✕"
                    color: Theme.muted
                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: HermesApp.error = "" }
                }
            }
        }

        Composer {
            id: composer
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: view.columnWidth
            Layout.topMargin: view.empty ? 0 : 8
            Layout.bottomMargin: view.empty ? 0 : 20
            botName: view.botName
            visible: HermesApp.connected || !view.empty
        }

        Item { Layout.fillHeight: true; visible: view.empty }
    }

    // Copying goes through a hidden editor; QML has no clipboard API.
    TextEdit { id: clipboard; visible: false }
}
