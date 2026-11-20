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
    // The jobs panel docks beside the chat when there's room, and slides over
    // it in a narrow window rather than squeezing the conversation.
    readonly property int panelWidth: 320
    readonly property bool panelDocked: width - panelWidth >= 480
    readonly property bool panelShown: sidePanel.open && bot !== null
    // 0 → 1 as the panel slides in; the layout follows it frame by frame.
    property real panelProgress: panelShown ? 1 : 0
    Behavior on panelProgress { NumberAnimation { duration: 240; easing.type: Easing.OutCubic } }
    readonly property real panelSpace: panelDocked ? panelWidth * panelProgress : 0
    readonly property int columnWidth: Math.max(220, Math.min(width - panelSpace - 48, 780))
    // New rows animate in; a whole transcript loading at once doesn't.
    property bool animateAdds: false

    // Every row carries every role so the ListModel keeps one schema.
    ListModel { id: messages }

    property int assistantRow: -1
    // This turn's to-do card, updated in place by todo.updated.
    property int todoRow: -1
    property var thinkingSince: 0

    function addRow(fields) {
        messages.append(Object.assign({
            role: "assistant", text: "", reasoning: "", toolId: "", name: "",
            detail: "", summary: "", done: true, thoughtMs: 0, attachments: "[]",
            args: "", result: "", diff: "", todos: "[]", duration: 0, error: "", timestamp: 0, rowId: -1
        }, fields))
        if (messages.get(messages.count - 1).role === "user")
            lastUserIndex = messages.count - 1
        return messages.count - 1
    }

    // The latest message you sent: it can be edited, and the answers after
    // it regenerated.
    property int lastUserIndex: -1

    function timeLabel(ts) {
        if (!ts) return ""
        const d = new Date(ts * 1000)
        const today = new Date()
        return d.toDateString() === today.toDateString()
            ? d.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
            : d.toLocaleString(Qt.locale(), Locale.ShortFormat)
    }

    // Drops your latest message and everything after it, then sends `text`
    // in its place (regenerate when unchanged, edit otherwise).
    function resendLast(text) {
        const i = lastUserIndex
        if (i < 0 || HermesApp.streaming) return
        const rowId = messages.get(i).rowId
        if (rowId < 0) {
            HermesApp.error = "This message can't be changed yet; reopen the chat and try again."
            return
        }
        messages.remove(i, messages.count - i)
        lastUserIndex = -1
        assistantRow = -1
        todoRow = -1
        HermesApp.resend(text, rowId)
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
            view.animateAdds = false
            chatFade.restart()
            messages.clear()
            view.assistantRow = -1
            view.todoRow = -1
            view.lastUserIndex = -1
            view.thinkingSince = 0
            for (const row of JSON.parse(json))
                view.addRow(Object.assign({ done: true }, row))
            view.scrollToEnd()
            composer.focusInput()
            Qt.callLater(() => view.animateAdds = true)
        }
        function onUserMessage(text, attachments) {
            view.addRow({ role: "user", text: text, attachments: attachments, timestamp: Date.now() / 1000 })
            view.assistantRow = -1
            view.todoRow = -1
            view.scrollToEnd()
        }
        function onUserRowId(rowId) {
            if (view.lastUserIndex >= 0)
                messages.setProperty(view.lastUserIndex, "rowId", rowId)
        }
        function onSteered(text, status) {
            view.addRow({ role: "info", text: status === "rejected" ? "Couldn't steer: " + text : "↪ " + text })
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
        function onToolStarted(json) {
            const tool = JSON.parse(json)
            view.closeThinking()
            if (view.assistantRow >= 0)
                messages.setProperty(view.assistantRow, "done", true)
            view.addRow(Object.assign({ role: "tool", done: false }, tool))
            // Text after a tool call starts a new answer below it.
            view.assistantRow = -1
            view.scrollToEnd()
        }
        function onToolFinished(json) {
            const tool = JSON.parse(json)
            for (let i = messages.count - 1; i >= 0; i--) {
                const row = messages.get(i)
                if (row.role === "tool" && !row.done && (row.toolId === tool.toolId || (!tool.toolId && row.name === tool.name))) {
                    for (const k of ["result", "diff", "todos", "duration", "error", "summary"])
                        if (tool[k] !== undefined)
                            messages.setProperty(i, k, tool[k])
                    messages.setProperty(i, "done", true)
                    break
                }
            }
        }
        function onAssistantInterim(text, alreadyStreamed) {
            if (!alreadyStreamed && text.length > 0) {
                view.ensureAssistantRow()
                messages.setProperty(view.assistantRow, "text", messages.get(view.assistantRow).text + text)
            }
            // The commentary is its own section; what follows starts a new one.
            if (view.assistantRow >= 0)
                messages.setProperty(view.assistantRow, "done", true)
            view.assistantRow = -1
        }
        function onTodosUpdated(json) {
            if (view.todoRow >= 0 && view.todoRow < messages.count && messages.get(view.todoRow).role === "todos") {
                messages.setProperty(view.todoRow, "todos", json)
            } else {
                view.todoRow = view.addRow({ role: "todos", todos: json })
                view.scrollToEnd()
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
            visible: view.bot !== null

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

                // How full the model's context is.
                Item {
                    id: gauge
                    readonly property var usage: {
                        try { return HermesApp.usage ? JSON.parse(HermesApp.usage) : null } catch (e) { return null }
                    }
                    visible: usage !== null && usage.contextMax > 0
                    Layout.preferredWidth: 120
                    Layout.preferredHeight: 30
                    function k(n) { return n >= 1000 ? (n / 1000).toFixed(n >= 10000 ? 0 : 1) + "k" : Math.round(n) }
                    Column {
                        anchors.verticalCenter: parent.verticalCenter
                        width: parent.width
                        spacing: 3
                        Label {
                            text: gauge.usage ? Math.round(gauge.usage.percent) + "% context" : ""
                            color: Theme.muted
                            font.pixelSize: 11
                        }
                        Rectangle {
                            width: parent.width
                            height: 3
                            radius: 1.5
                            color: Theme.border
                            Rectangle {
                                height: parent.height
                                radius: 1.5
                                width: gauge.usage ? parent.width * Math.min(1, gauge.usage.percent / 100) : 0
                                color: gauge.usage && gauge.usage.percent > 85 ? Theme.danger : Theme.accent
                                Behavior on width { NumberAnimation { duration: 300 } }
                            }
                        }
                    }
                    HoverHandler { id: gaugeHover }
                    ToolTip.visible: gaugeHover.hovered && usage !== null
                    ToolTip.text: usage ? gauge.k(usage.contextUsed) + " / " + gauge.k(usage.contextMax) + " tokens in context"
                                          + (usage.cost >= 0 ? "\nCost so far: $" + usage.cost.toFixed(4) : "") : ""
                }

                // Side panel toggles.
                Repeater {
                    model: [
                        { tab: "jobs", label: "Scheduled", count: sidePanel.jobs.length },
                        { tab: "screen", label: "Screen", count: 0 }
                    ]
                    delegate: Rectangle {
                        id: toggle
                        required property var modelData
                        readonly property bool on: sidePanel.open && sidePanel.tab === modelData.tab
                        Layout.preferredHeight: 30
                        Layout.preferredWidth: toggleLabel.implicitWidth + 24
                        radius: Theme.radius
                        color: on ? Theme.raised : (toggleHover.hovered ? Qt.alpha(Theme.raised, 0.6) : Qt.alpha(Theme.raised, 0))
                        Behavior on color { ColorAnimation { duration: 140 } }
                        border.width: 1
                        border.color: on ? Qt.alpha(Theme.accent, 0.5) : Theme.border
                        Behavior on border.color { ColorAnimation { duration: 140 } }
                        Label {
                            id: toggleLabel
                            anchors.centerIn: parent
                            text: toggle.modelData.label + (toggle.modelData.count > 0 ? "  " + toggle.modelData.count : "")
                            color: toggle.on ? Theme.foreground : Theme.muted
                            font.pixelSize: 13
                        }
                        HoverHandler { id: toggleHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: sidePanel.show(toggle.modelData.tab) }
                    }
                }
            }
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

        ColumnLayout {
            id: chatArea
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

        // The conversation, with the loading and empty states centered over
        // the same area so they never depend on the column layout.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

        ListView {
            id: conversation
            anchors.fill: parent
            visible: !HermesApp.chatLoading
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

            add: Transition {
                enabled: view.animateAdds
                NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 220; easing.type: Easing.OutCubic }
                NumberAnimation { property: "scale"; from: 0.97; to: 1; duration: 220; easing.type: Easing.OutCubic }
            }

            NumberAnimation on opacity {
                id: chatFade
                from: 0
                to: 1
                duration: 260
                easing.type: Easing.OutCubic
                running: false
            }
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

            delegate: DelegateChooser {
                role: "role"

                DelegateChoice {
                    roleValue: "user"
                    Item {
                        id: userRow
                        required property int index
                        required property real timestamp
                        required property string text
                        required property string attachments
                        HoverHandler { id: userHover }

                        // Edit (latest message only) and time, beside the bubble on hover.
                        Row {
                            visible: userRow.text.length > 0
                            anchors.verticalCenter: bubble.verticalCenter
                            x: bubble.x - width - 12
                            spacing: 12
                            opacity: userHover.hovered ? 1 : 0
                            Behavior on opacity { NumberAnimation { duration: 150 } }
                            Label {
                                visible: userRow.timestamp > 0
                                text: view.timeLabel(userRow.timestamp)
                                color: Theme.muted
                                font.pixelSize: 12
                            }
                            Label {
                                visible: userRow.index === view.lastUserIndex && !HermesApp.streaming
                                text: "Edit"
                                color: editHover.hovered ? Theme.foreground : Theme.muted
                                font.pixelSize: 12
                                HoverHandler { id: editHover; cursorShape: Qt.PointingHandCursor }
                                TapHandler { onTapped: composer.startEdit(userRow.text) }
                            }
                        }
                        readonly property var files: {
                            try { return JSON.parse(attachments || "[]") } catch (e) { return [] }
                        }
                        readonly property real rightEdge: (width + view.columnWidth) / 2
                        width: ListView.view.width
                        height: (files.length > 0 ? sent.height + (userRow.text.length > 0 ? 6 : 0) : 0)
                                + (userRow.text.length > 0 ? bubble.height : 0)

                        // What was attached, right-aligned above the text.
                        Flow {
                            id: sent
                            visible: userRow.files.length > 0
                            width: Math.min(implicitWidth, view.columnWidth * 0.8)
                            x: userRow.rightEdge - width
                            layoutDirection: Qt.RightToLeft
                            spacing: 6
                            Repeater {
                                model: userRow.files
                                delegate: Loader {
                                    required property var modelData
                                    sourceComponent: modelData.kind === "image" ? imageChip : fileChip
                                    Component {
                                        id: imageChip
                                        RemoteImage {
                                            url: modelData.url || ""
                                            path: modelData.path || ""
                                            maxWidth: 240
                                            maxHeight: 180
                                            onClicked: Qt.openUrlExternally(source)
                                        }
                                    }
                                    Component {
                                        id: fileChip
                                        Rectangle {
                                            width: fileLabel.implicitWidth + 36
                                            height: 34
                                            radius: Theme.radius
                                            color: Theme.raised
                                            border.width: 1
                                            border.color: Theme.border
                                            Row {
                                                anchors.centerIn: parent
                                                spacing: 6
                                                Label { text: "📄"; font.pixelSize: 13 }
                                                Label {
                                                    id: fileLabel
                                                    text: modelData.name
                                                    color: Theme.foreground
                                                    font.pixelSize: 12
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        Rectangle {
                            id: bubble
                            visible: userRow.text.length > 0
                            readonly property real maxWidth: view.columnWidth * 0.8
                            x: userRow.rightEdge - width
                            y: userRow.files.length > 0 ? sent.height + 6 : 0
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
                        required property real timestamp
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
                                visible: Layout.preferredHeight > 0
                                Layout.fillWidth: true
                                Layout.preferredHeight: botRow.showReasoning && botRow.reasoning.length > 0 ? reasoningText.height : 0
                                Behavior on Layout.preferredHeight { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
                                clip: true
                                opacity: botRow.showReasoning ? 1 : 0
                                Behavior on opacity { NumberAnimation { duration: 200 } }
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

                            MarkdownView {
                                Layout.fillWidth: true
                                visible: botRow.text.length > 0
                                markdown: botRow.text
                            }

                            // Copy and regenerate, once the answer is complete.
                            Row {
                                visible: botRow.done && botRow.text.length > 0
                                spacing: 14
                                opacity: rowHover.hovered ? 1 : 0
                                Behavior on opacity { NumberAnimation { duration: 150 } }
                                Label {
                                    id: copyLabel
                                    text: copied ? "Copied" : "Copy"
                                    property bool copied: false
                                    color: copyHover.hovered ? Theme.foreground : Theme.muted
                                    font.pixelSize: 12
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
                                Label {
                                    visible: botRow.index > view.lastUserIndex && view.lastUserIndex >= 0 && !HermesApp.streaming
                                    text: "Regenerate"
                                    color: regenHover.hovered ? Theme.foreground : Theme.muted
                                    font.pixelSize: 12
                                    HoverHandler { id: regenHover; cursorShape: Qt.PointingHandCursor }
                                    TapHandler { onTapped: view.resendLast(messages.get(view.lastUserIndex).text) }
                                }
                                Label {
                                    visible: botRow.timestamp > 0
                                    text: view.timeLabel(botRow.timestamp)
                                    color: Theme.muted
                                    font.pixelSize: 12
                                }
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
                        required property string args
                        required property string result
                        required property string diff
                        required property string todos
                        required property real duration
                        required property string error
                        required property bool done
                        width: ListView.view.width
                        height: toolCard.implicitHeight
                        ToolCard {
                            id: toolCard
                            x: (toolRow.width - view.columnWidth) / 2
                            width: view.columnWidth
                            name: toolRow.name
                            detail: toolRow.detail
                            args: toolRow.args
                            result: toolRow.result
                            diff: toolRow.diff
                            todos: toolRow.todos
                            duration: toolRow.duration
                            error: toolRow.error
                            done: toolRow.done
                        }
                    }
                }

                DelegateChoice {
                    roleValue: "todos"
                    Item {
                        id: todoRowItem
                        required property string todos
                        width: ListView.view.width
                        height: todoCard.implicitHeight
                        ToolCard {
                            id: todoCard
                            x: (todoRowItem.width - view.columnWidth) / 2
                            width: view.columnWidth
                            name: "todo"
                            detail: "Plan"
                            todos: todoRowItem.todos
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

        Item {
            anchors.fill: parent

            BusyIndicator {
                anchors.centerIn: parent
                running: HermesApp.chatLoading
                visible: running
                width: 40
                height: 40
            }

            // An empty Bot Chat: introduce the bot.
            Column {
                anchors.centerIn: parent
                width: view.columnWidth
                spacing: 10
                visible: opacity > 0
                opacity: view.empty && !HermesApp.chatLoading ? 1 : 0
                Behavior on opacity { NumberAnimation { duration: 200 } }

                BotAvatar {
                    anchors.horizontalCenter: parent.horizontalCenter
                    name: view.botName
                    size: 64
                    visible: view.bot !== null
                }
                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    text: view.bot ? view.botName : (HermesApp.connected ? "Pick a bot" : "Not connected")
                    color: Theme.foreground
                    font.pixelSize: 28
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    text: view.bot ? (view.bot.description || "Say hello to start your chat.") : HermesApp.status
                    color: Theme.muted
                    font.pixelSize: 15
                }
            }
        }
        }

        // Errors stay until dismissed or the next action clears them.
        Rectangle {
            readonly property bool shown: HermesApp.error.length > 0
            visible: Layout.preferredHeight > 0.5
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: view.columnWidth
            Layout.preferredHeight: shown ? errorText.implicitHeight + 20 : 0
            Behavior on Layout.preferredHeight { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
            opacity: shown ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: 200 } }
            clip: true
            Layout.bottomMargin: shown ? 8 : 0
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
            Layout.topMargin: 8
            Layout.bottomMargin: 20
            botName: view.botName
            bot: view.bot
            onEditSubmitted: text => view.resendLast(text)
            visible: HermesApp.connected
        }

        }

        // Holds the docked panel's place.
        Item {
            Layout.fillHeight: true
            Layout.preferredWidth: view.panelSpace
        }
        }
    }

    // The bot's screen, expanded over the conversation.
    Rectangle {
        id: expandedScreen
        x: 0
        y: 56
        width: view.width - view.panelSpace
        height: view.height - 56
        color: "black"
        visible: opacity > 0
        opacity: sidePanel.screenExpanded ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
        z: 5

        Rectangle {
            anchors.top: parent.top
            anchors.right: parent.right
            anchors.margins: 12
            z: 10
            width: collapseLabel.implicitWidth + 24
            height: 30
            radius: Theme.radius
            color: Qt.alpha(Theme.surface, collapseHover.hovered ? 0.95 : 0.75)
            border.width: 1
            border.color: Theme.border
            Label {
                id: collapseLabel
                anchors.centerIn: parent
                text: "Collapse screen"
                color: Theme.foreground
                font.pixelSize: 13
            }
            HoverHandler { id: collapseHover; cursorShape: Qt.PointingHandCursor }
            TapHandler { onTapped: sidePanel.screenExpanded = false }
        }
    }

    SidePanel {
        id: sidePanel
        expandedHost: expandedScreen
        z: 6
        x: view.width - width * view.panelProgress
        anchors.top: parent.top
        anchors.topMargin: 56
        anchors.bottom: parent.bottom
        width: view.panelDocked ? view.panelWidth : Math.min(view.panelWidth, view.width - 40)
        visible: view.panelProgress > 0.001 && view.bot !== null

        // Floating over the chat: a shadow edge so it reads as a layer.
        Rectangle {
            visible: !view.panelDocked
            anchors.right: parent.left
            width: 12
            height: parent.height
            gradient: Gradient {
                orientation: Gradient.Horizontal
                GradientStop { position: 0; color: "transparent" }
                GradientStop { position: 1; color: Qt.alpha("black", 0.35) }
            }
        }
    }

    // Drop files anywhere on the chat to attach them.
    DropArea {
        anchors.fill: parent
        z: 20
        enabled: HermesApp.connected && view.bot !== null
        onEntered: drag => drag.accept(Qt.CopyAction)
        onDropped: drop => {
            if (drop.hasUrls) {
                HermesApp.addFiles(JSON.stringify(drop.urls.map(u => u.toString())))
                drop.accept(Qt.CopyAction)
            }
        }
        Rectangle {
            anchors.fill: parent
            visible: parent.containsDrag
            color: Qt.alpha(Theme.background, 0.85)
            border.width: 2
            border.color: Theme.accent
            Label {
                anchors.centerIn: parent
                text: "Drop files to attach"
                color: Theme.foreground
                font.pixelSize: 20
            }
        }
    }

    // Copying goes through a hidden editor; QML has no clipboard API.
    TextEdit { id: clipboard; visible: false }
}
