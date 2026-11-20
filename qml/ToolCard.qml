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
    readonly property bool hasBody: args.length > 0 || result.length > 0 || diff.length > 0 || error.length > 0

    Rectangle {
        Layout.maximumWidth: card.width
        Layout.preferredWidth: chip.implicitWidth + 20
        Layout.preferredHeight: 30
        radius: Theme.radius
        color: chipHover.hovered ? Theme.raised : Qt.alpha(Theme.raised, 0)
        Behavior on color { ColorAnimation { duration: 120 } }
        border.width: 1
        border.color: card.error.length > 0 ? Qt.alpha(Theme.danger, 0.6) : Theme.border
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
                color: card.error.length > 0 ? Theme.danger : Theme.muted
                font.pixelSize: 12
            }
            Label {
                text: card.name || "tool"
                color: Theme.foreground
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
                text: (modelData.status === "completed" ? "☑ " : modelData.status === "in_progress" ? "◐ "
                       : modelData.status === "cancelled" ? "☒ " : "☐ ") + modelData.content
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
                    Label { text: parent.title; color: Theme.muted; font.pixelSize: 11; font.weight: Font.DemiBold }
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

                Section { title: "Error"; body: card.error; tint: Theme.danger }
                Section { title: "Arguments"; body: card.args }
                Section { title: "Changes"; body: card.diff; isDiff: true }
                Section { title: "Result"; body: card.result }
            }
        }
    }

    // A unified diff with added lines green and removed lines red.
    function diffHtml(text) {
        const esc = s => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/ /g, "&nbsp;")
        return text.split("\n").map(line => {
            if (line.startsWith("+++") || line.startsWith("---"))
                return "<span style=\"color:" + Theme.muted + "\">" + esc(line) + "</span>"
            if (line.startsWith("+"))
                return "<span style=\"color:#3fb950\">" + esc(line) + "</span>"
            if (line.startsWith("-"))
                return "<span style=\"color:" + Theme.danger + "\">" + esc(line) + "</span>"
            if (line.startsWith("@@"))
                return "<span style=\"color:#4493f8\">" + esc(line) + "</span>"
            return esc(line)
        }).join("<br>")
    }
}
