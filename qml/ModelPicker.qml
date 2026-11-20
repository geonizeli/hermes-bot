import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// Searchable list of the server's models, grouped by provider. A pick only
// changes this Bot Chat; the bot's own default stays as it is.
Popup {
    id: picker
    property var providers: []
    property string currentModel
    property string currentProvider

    height: Math.min(460, list.contentHeight + search.height + 40)
    padding: 8
    modal: false
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    background: Rectangle {
        color: Theme.surface
        radius: Theme.radius
        border.width: 1
        border.color: Theme.border
    }

    enter: Transition {
        NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 140 }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; from: 1; to: 0; duration: 100 }
    }

    onOpened: { search.text = ""; search.forceActiveFocus() }

    // One row per (provider, model), filtered by the search text.
    readonly property var rows: {
        const q = search.text.trim().toLowerCase()
        const out = []
        for (const p of providers) {
            for (const m of (p.models || [])) {
                const id = typeof m === "string" ? m : (m.id || m.name || "")
                if (!id) continue
                if (q && id.toLowerCase().indexOf(q) < 0 && p.name.toLowerCase().indexOf(q) < 0)
                    continue
                out.push({ model: id, provider: p.slug, providerName: p.name + (p.authenticated ? "" : "  (not signed in)") })
            }
        }
        return out
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 6

        TextField {
            id: search
            Layout.fillWidth: true
            placeholderText: "Search models"
            Keys.onReturnPressed: if (picker.rows.length > 0) list.pick(picker.rows[0])
            Keys.onDownPressed: list.forceActiveFocus()
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: picker.rows
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
            section.property: "providerName"
            section.delegate: Label {
                required property string section
                width: ListView.view.width
                text: section
                color: Theme.muted
                font.pixelSize: 11
                font.weight: Font.DemiBold
                font.capitalization: Font.AllUppercase
                topPadding: 8
                bottomPadding: 4
                leftPadding: 8
            }

            function pick(row) {
                HermesApp.setModel(row.model, row.provider, false)
                picker.close()
            }

            delegate: Rectangle {
                id: row
                required property var modelData
                readonly property bool current: modelData.model === picker.currentModel
                    && (!picker.currentProvider || modelData.provider === picker.currentProvider)
                width: ListView.view.width
                height: 32
                radius: Theme.radius
                color: rowHover.hovered ? Theme.raised : Qt.alpha(Theme.raised, 0)
                Behavior on color { ColorAnimation { duration: 100 } }
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    Label {
                        Layout.fillWidth: true
                        text: row.modelData.model
                        color: Theme.foreground
                        font.pixelSize: 13
                        font.weight: row.current ? Font.DemiBold : Font.Normal
                        elide: Text.ElideRight
                    }
                    Label {
                        visible: row.current
                        text: "✓"
                        color: Theme.accent
                    }
                }
                HoverHandler { id: rowHover; cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: list.pick(row.modelData) }
            }

            Label {
                anchors.centerIn: parent
                visible: list.count === 0
                text: HermesApp.modelsLoading ? "Loading models…" : "No models match"
                color: Theme.muted
            }
        }
    }
}
