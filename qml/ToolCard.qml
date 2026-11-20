import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// One tool call: a compact chip (status, name, what it's doing, duration)
// that expands into its arguments, result, diff and to-do list.
ColumnLayout {
    id: card
    property string name
    property string detail
    property string args
    property string result
    property string diff
    property string todos: "[]"
    property real duration: 0
    property string error
    property bool done: true
    property bool expanded: false
    spacing: 6

    readonly property var todoList: {
        try { return JSON.parse(todos || "[]") } catch (e) { return [] }
    }
    // Each kind of tool gets its own colour, so a run of calls reads at a glance.
    readonly property color tint: {
        const n = name.toLowerCase()
        if (/terminal|shell|exec|code|process|bash|run/.test(n)) return Theme.green
        if (/patch|write|edit|file|read|search_files|fs/.test(n)) return Theme.blue
        if (/web|browser|fetch|url|search/.test(n)) return Theme.cyan
        if (/image|vision|media|screen|display/.test(n)) return Theme.magenta
        if (/todo|memory|skill|cron|schedule/.test(n)) return Theme.yellow
        if (/delegate|agent|session|send|message/.test(n)) return Theme.orange
        return Theme.accent
    }
    readonly property bool hasBody: args.length > 0 || result.length > 0 || diff.length > 0 || error.length > 0

    Rectangle {
        Layout.maximumWidth: card.width
        Layout.preferredWidth: chip.implicitWidth + 20
        Layout.preferredHeight: 30
        radius: Theme.radius
        color: chipHover.hovered ? Qt.alpha(card.tint, 0.16) : Qt.alpha(card.tint, 0.07)
        Behavior on color { ColorAnimation { duration: 120 } }
        border.width: 1
        border.color: card.error.length > 0 ? Qt.alpha(Theme.danger, 0.6) : Qt.alpha(card.tint, 0.35)
        clip: true

        RowLayout {
            id: chip
            anchors.verticalCenter: parent.verticalCenter
            x: 10
            width: Math.min(implicitWidth, card.width - 20)
            spacing: 8
            BusyIndicator {
                visible: !card.done
                running: visible
                Layout.preferredWidth: 14
                Layout.preferredHeight: 14
            }
            Label {
                visible: card.done
                text: card.error.length > 0 ? "✕" : "✓"
                color: card.error.length > 0 ? Theme.danger : card.tint
                font.pixelSize: 12
            }
            Label {
                text: card.name || "tool"
                color: card.tint
                font.pixelSize: 13
                font.family: "monospace"
            }
            Label {
                visible: card.detail.length > 0
                Layout.fillWidth: true
                Layout.maximumWidth: 420
                text: card.detail
                color: Theme.muted
                font.pixelSize: 12
                elide: Text.ElideRight
            }
            Label {
                visible: card.duration > 0
                text: card.duration < 10 ? card.duration.toFixed(1) + "s" : Math.round(card.duration) + "s"
                color: Theme.muted
                font.pixelSize: 11
            }
            Label {
                visible: card.hasBody
                text: card.expanded ? "▾" : "▸"
                color: Theme.muted
                font.pixelSize: 11
            }
        }
        HoverHandler { id: chipHover; cursorShape: card.hasBody ? Qt.PointingHandCursor : Qt.ArrowCursor }
        TapHandler { enabled: card.hasBody; onTapped: card.expanded = !card.expanded }
    }

    // To-dos show without expanding: they're the point of the call.
    Column {
        visible: card.todoList.length > 0
        Layout.fillWidth: true
        leftPadding: 12
        spacing: 2
        Repeater {
            model: card.todoList
            delegate: Label {
                required property var modelData
                textFormat: Text.StyledText
                readonly property var mark: ({ completed: ["☑", Theme.green], in_progress: ["◐", Theme.yellow],
                                               cancelled: ["☒", Theme.red] })[modelData.status] || ["☐", Theme.muted]
                text: "<font color=\"" + mark[1] + "\">" + mark[0] + "</font>&nbsp;" + card.htmlEscape(modelData.content)
                color: modelData.status === "completed" || modelData.status === "cancelled" ? Theme.muted : Theme.foreground
                font.pixelSize: 13
                font.strikeout: modelData.status === "cancelled"
            }
        }
    }

    // Expanded details.
    Rectangle {
        id: body
        readonly property bool shown: card.expanded && card.hasBody
        visible: Layout.preferredHeight > 0.5
        Layout.fillWidth: true
        Layout.preferredHeight: shown ? Math.min(details.implicitHeight + 20, 520) : 0
        Behavior on Layout.preferredHeight { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
        opacity: shown ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: 200 } }
        radius: Theme.radius
        color: Theme.surface
        border.width: 1
        border.color: Theme.border
        clip: true

        Flickable {
            anchors.fill: parent
            anchors.margins: 10
            contentHeight: details.implicitHeight
            contentWidth: width
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

            Column {
                id: details
                width: parent.width
                spacing: 8

                component Section: Column {
                    property string title
                    property string body
                    property bool isDiff: false
                    property color tint: Theme.foreground
                    visible: body.length > 0
                    width: details.width
                    spacing: 4
                    property color heading: Theme.muted
                    Label { text: parent.title; color: parent.heading; font.pixelSize: 11; font.weight: Font.DemiBold }
                    TextEdit {
                        width: parent.width
                        readOnly: true
                        selectByMouse: true
                        wrapMode: TextEdit.WrapAnywhere
                        font.family: "monospace"
                        font.pixelSize: 12
                        color: parent.tint
                        selectionColor: Qt.alpha(Theme.accent, 0.4)
                        textFormat: parent.isDiff ? TextEdit.RichText : TextEdit.PlainText
                        text: parent.isDiff ? card.diffHtml(parent.body) : parent.body
                    }
                }

                Section { title: "Error"; body: card.error; tint: Theme.danger; heading: Theme.danger }
                Section { title: "Arguments"; body: card.args; heading: card.tint }
                Section { title: "Changes"; body: card.diff; isDiff: true; heading: card.tint }
                Section { title: "Result"; body: card.result; heading: card.tint }
            }
        }
    }

    function htmlEscape(s) {
        return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    }

    // A unified diff with added lines green and removed lines red.
    function diffHtml(text) {
        const esc = s => card.htmlEscape(s).replace(/ /g, "&nbsp;")
        return text.split("\n").map(line => {
            if (line.startsWith("+++") || line.startsWith("---"))
                return "<span style=\"color:" + Theme.muted + "\">" + esc(line) + "</span>"
            if (line.startsWith("+"))
                return "<span style=\"color:" + Theme.green + "\">" + esc(line) + "</span>"
            if (line.startsWith("-"))
                return "<span style=\"color:" + Theme.red + "\">" + esc(line) + "</span>"
            if (line.startsWith("@@"))
                return "<span style=\"color:" + Theme.blue + "\">" + esc(line) + "</span>"
            return esc(line)
        }).join("<br>")
    }
}
