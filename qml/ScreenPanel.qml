import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// The bot's own computer (Hermes Bot Screen): status, start/stop, install,
// and the live view, which can expand over the chat.
Rectangle {
    id: panel
    color: Theme.surface

    property bool shown: false
    property Item expandedHost: null
    property bool expanded: false
    signal closeRequested()

    readonly property var status: {
        try { return HermesApp.screen ? JSON.parse(HermesApp.screen) : null } catch (e) { return null }
    }
    readonly property bool running: !!(status && status.running)
    readonly property var lease: status && status.lease ? status.lease : null
    readonly property bool botInControl: !lease || lease.holder !== "human"

    onStatusChanged: if (lease) screen.applyLease(JSON.stringify(lease))
    onRunningChanged: if (!running) expanded = false
    onShownChanged: if (shown) HermesApp.refreshScreen()

    // Connect only while someone can see it.
    readonly property string runKey: running && (shown || expanded) ? HermesApp.activeBot : ""
    onRunKeyChanged: {
        screen.detach()
        if (runKey.length > 0) {
            screen.profile = runKey
            screen.attach()
        }
    }

    // Keeps the status fresh while visible (lease changes, idle stop).
    Timer {
        interval: 15000
        repeat: true
        running: panel.shown && HermesApp.connected
        onTriggered: HermesApp.refreshScreen()
    }

    component PanelButton: Rectangle {
        id: btn
        property string text
        property bool primary: false
        property bool busy: false
        signal clicked()
        implicitWidth: label.implicitWidth + 24
        implicitHeight: 30
        radius: Theme.radius
        opacity: enabled ? 1 : 0.5
        color: primary ? (hover.hovered ? Qt.lighter(Theme.accent, 1.1) : Theme.accent)
                       : (hover.hovered ? Theme.raised : Qt.alpha(Theme.raised, 0.5))
        border.width: primary ? 0 : 1
        border.color: Theme.border
        Behavior on color { ColorAnimation { duration: 120 } }
        Label {
            id: label
            anchors.centerIn: parent
            text: btn.busy ? btn.text + "…" : btn.text
            color: btn.primary ? Theme.accentForeground : Theme.foreground
            font.pixelSize: 13
        }
        HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
        TapHandler { enabled: btn.enabled && !btn.busy; onTapped: btn.clicked() }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            Layout.leftMargin: 16
            Layout.rightMargin: 10
            spacing: 6
            Label {
                text: "Bot screen"
                color: Theme.foreground
                font.pixelSize: 14
                font.weight: Font.DemiBold
            }
            BusyIndicator {
                running: HermesApp.screenBusy
                visible: running
                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
            }
            Item { Layout.fillWidth: true }
            Repeater {
                model: [
                    { glyph: "↻", tip: "Refresh", act: () => HermesApp.refreshScreen() },
                    { glyph: "✕", tip: "Hide", act: () => panel.closeRequested() }
                ]
                delegate: Rectangle {
                    required property var modelData
                    width: 28
                    height: 28
                    radius: Theme.radius
                    color: Qt.alpha(Theme.raised, btnHover.hovered ? 1 : 0)
                    Behavior on color { ColorAnimation { duration: 120 } }
                    Label {
                        anchors.centerIn: parent
                        text: modelData.glyph
                        color: Theme.muted
                        font.pixelSize: 14
                    }
                    ToolTip.visible: btnHover.hovered
                    ToolTip.text: modelData.tip
                    ToolTip.delay: 600
                    HoverHandler { id: btnHover; cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: modelData.act() }
                }
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.border }

        Flickable {
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentHeight: body.implicitHeight + 32
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

            ColumnLayout {
                id: body
                x: 16
                y: 16
                width: parent.width - 32
                spacing: 12

                // ---- states without a running screen ----
                Label {
                    Layout.fillWidth: true
                    visible: text.length > 0
                    wrapMode: Text.Wrap
                    color: panel.status && panel.status.error ? Theme.danger : Theme.muted
                    font.pixelSize: 13
                    text: {
                        const s = panel.status
                        if (!HermesApp.connected) return "Not connected."
                        if (!s) return "Checking the screen…"
                        if (s.unavailable) return "This Hermes server doesn't have bot screens. Update Hermes to use them."
                        if (s.error) return "Couldn't check the screen: " + s.error
                        if (s.supported === false) return "Bot screens need a Linux server."
                        if (s.installed === false) return "The screen isn't installed on the server yet."
                            + (s.missing && s.missing.length ? " Missing: " + s.missing.join(", ") + "." : "")
                        if (s.blocker) return s.blocker
                        if (!s.running) return "The screen is off. The bot turns it on when it needs a browser or desktop app, or you can start it now."
                        return ""
                    }
                }

                // Install: the command, a button, and its output.
                TextArea {
                    Layout.fillWidth: true
                    visible: !!(panel.status && panel.status.installed === false && panel.status.install_command)
                    text: panel.status && panel.status.install_command ? panel.status.install_command : ""
                    readOnly: true
                    wrapMode: TextEdit.WrapAnywhere
                    font.family: "monospace"
                    font.pixelSize: 12
                    color: Theme.foreground
                    background: Rectangle { color: Theme.background; radius: Theme.radius; border.color: Theme.border }
                }
                PanelButton {
                    visible: !!(panel.status && panel.status.installed === false && panel.status.supported !== false
                                && String(panel.status.placement || "gateway") === "gateway")
                    text: "Install on the server"
                    primary: true
                    busy: HermesApp.screenBusy
                    onClicked: HermesApp.installScreen()
                }
                Label {
                    Layout.fillWidth: true
                    visible: HermesApp.screenLog.length > 0
                    text: HermesApp.screenLog
                    color: Theme.muted
                    font.family: "monospace"
                    font.pixelSize: 11
                    wrapMode: Text.WrapAnywhere
                    maximumLineCount: 40
                    elide: Text.ElideLeft
                }

                PanelButton {
                    visible: !!(panel.status && panel.status.installed !== false && panel.status.supported !== false
                                && !panel.status.running && !panel.status.unavailable && !panel.status.error)
                    text: HermesApp.screenBusy ? "Starting" : "Start screen"
                    primary: true
                    busy: HermesApp.screenBusy
                    onClicked: HermesApp.startScreen()
                }

                // ---- the live screen ----
                Rectangle {
                    id: smallHolder
                    visible: panel.running
                    Layout.fillWidth: true
                    Layout.preferredHeight: width * (screen.frameHeight > 0 ? screen.frameHeight / screen.frameWidth : 0.625)
                    color: "black"
                    radius: Theme.radius
                    clip: true

                    Label {
                        anchors.centerIn: parent
                        visible: panel.expanded
                        text: "Showing over the chat"
                        color: Theme.muted
                        font.pixelSize: 12
                    }
                    TapHandler {
                        enabled: !panel.expanded && !screen.inControl
                        onDoubleTapped: panel.expanded = true
                    }
                }

                Label {
                    Layout.fillWidth: true
                    visible: panel.running
                    wrapMode: Text.Wrap
                    font.pixelSize: 12
                    color: screen.inControl ? Theme.accent : Theme.muted
                    text: screen.state === "connecting" ? "Connecting to the screen…"
                          : screen.state === "ended" ? (screen.message || "Disconnected.")
                          : screen.inControl ? "You're in control. The bot waits until you give it back."
                          : panel.botInControl ? "Watching. The bot is in control."
                          : "Someone else is in control. Watching."
                }
                Label {
                    Layout.fillWidth: true
                    visible: panel.running && screen.state !== "ended" && screen.message.length > 0
                    text: screen.message
                    color: Theme.muted
                    font.pixelSize: 12
                    wrapMode: Text.Wrap
                }

                Flow {
                    Layout.fillWidth: true
                    visible: panel.running
                    spacing: 8
                    PanelButton {
                        text: panel.expanded ? "Collapse" : "Expand"
                        onClicked: panel.expanded = !panel.expanded
                    }
                    PanelButton {
                        text: screen.inControl ? "Give back" : "Take control"
                        primary: !screen.inControl
                        enabled: screen.state === "live"
                        onClicked: screen.inControl ? screen.giveBack() : screen.takeControl()
                    }
                    PanelButton {
                        visible: screen.state === "ended"
                        text: "Reconnect"
                        onClicked: { screen.detach(); screen.attach() }
                    }
                    PanelButton {
                        text: "Stop"
                        busy: HermesApp.screenBusy
                        onClicked: HermesApp.stopScreen(false)
                    }
                }
            }
        }
    }

    // The live view itself, moved between the panel and the expanded area.
    ScreenView {
        id: screen
        parent: panel.expanded && panel.expandedHost ? panel.expandedHost : smallHolder
        anchors.fill: parent
        visible: panel.running
        focus: inControl
        onInControlChanged: if (inControl) forceActiveFocus()

        BusyIndicator {
            anchors.centerIn: parent
            running: screen.state === "connecting"
            visible: running
        }

        MouseArea {
            anchors.fill: parent
            enabled: screen.inControl
            hoverEnabled: true
            acceptedButtons: Qt.AllButtons
            cursorShape: Qt.CrossCursor
            onPressed: mouse => { screen.forceActiveFocus(); screen.pointer(mouse.x, mouse.y, mouse.buttons) }
            onReleased: mouse => screen.pointer(mouse.x, mouse.y, mouse.buttons)
            onPositionChanged: mouse => screen.pointer(mouse.x, mouse.y, mouse.buttons)
            onWheel: wheel => screen.wheel(wheel.x, wheel.y, wheel.angleDelta.y)
        }

        Keys.onPressed: event => { if (screen.key(event.key, event.text, true)) event.accepted = true }
        Keys.onReleased: event => { if (screen.key(event.key, event.text, false)) event.accepted = true }

        // Accent edge while you're driving.
        Rectangle {
            anchors.fill: parent
            color: "transparent"
            border.width: screen.inControl ? 2 : 0
            border.color: Theme.accent
        }
    }
}
