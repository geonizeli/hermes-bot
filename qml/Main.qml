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
    title: activeBot ? activeBot.displayName + " — Hermes Bot" : "Hermes Bot"
    color: Theme.background

    // Default controls, colored from the Omarchy theme.
    palette.window: Theme.background
    palette.windowText: Theme.foreground
    palette.base: Theme.raised
    palette.alternateBase: Theme.surface
    palette.text: Theme.foreground
    palette.button: Theme.raised
    palette.buttonText: Theme.foreground
    palette.highlight: Theme.accent
    palette.highlightedText: Theme.accentForeground
    palette.placeholderText: Theme.muted
    palette.link: Theme.accent
    palette.mid: Theme.border
    palette.dark: Theme.border
    palette.light: Theme.raised
    palette.midlight: Theme.raised
    palette.shadow: Theme.surface
    palette.toolTipBase: Theme.raised
    palette.toolTipText: Theme.foreground
    font.pixelSize: 14

    readonly property var servers: JSON.parse(HermesApp.servers || "[]")
    readonly property var bots: JSON.parse(HermesApp.bots || "[]")
    readonly property var activeBot: bots.find(b => b.name === HermesApp.activeBot) || null

    Component.onCompleted: if (servers.length === 0) setupDialog.open()

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Sidebar {
            Layout.preferredWidth: 280
            Layout.fillHeight: true
            bots: win.bots
            servers: win.servers
            onManageServers: setupDialog.open()
            onEditBot: bot => editBotDialog.open_(bot)
        }

        ChatView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            bot: win.activeBot
            onEditBot: bot => editBotDialog.open_(bot)
        }
    }

    SetupDialog {
        id: setupDialog
        anchors.centerIn: parent
        servers: win.servers
    }

    EditBotDialog {
        id: editBotDialog
        anchors.centerIn: parent
    }

    RequestDialog {
        id: requestDialog
        anchors.centerIn: parent
    }

    Connections {
        target: HermesApp
        function onServerRequest(id, method, params) {
            requestDialog.open_(id, method, JSON.parse(params))
        }
    }
}
