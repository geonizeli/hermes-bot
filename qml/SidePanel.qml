import QtCore
import QtQuick
import QtQuick.Layouts
import HermesBot

// The chat's side panel: the bot's scheduled jobs or its screen. Whether it's
// open, and on which tab, is remembered between runs.
Rectangle {
    id: side
    color: Theme.surface

    property alias open: settings.open
    property alias tab: settings.tab
    // Where the screen goes when expanded over the chat.
    property Item expandedHost: null
    property alias screenExpanded: screenPanel.expanded
    readonly property var jobs: jobsPanel.jobs

    function show(which) {
        if (open && tab === which) {
            open = false
        } else {
            tab = which
            open = true
        }
    }

    Settings {
        id: settings
        category: "sidePanel"
        property bool open: true
        property string tab: "jobs"
    }

    Rectangle { width: 1; height: parent.height; color: Theme.border; z: 1 }

    StackLayout {
        anchors.fill: parent
        anchors.leftMargin: 1
        currentIndex: side.tab === "screen" ? 1 : 0

        JobsPanel {
            id: jobsPanel
            onCloseRequested: side.open = false
        }
        ScreenPanel {
            id: screenPanel
            shown: side.visible && side.open && side.tab === "screen"
            expandedHost: side.expandedHost
            onCloseRequested: side.open = false
        }
    }
}
