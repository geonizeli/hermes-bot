import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// The active bot's scheduled jobs (Hermes cron), beside the chat. Collapsible;
// whether it's open is remembered between runs.
Rectangle {
    id: panel
    color: Theme.surface

    property alias open: settings.jobsPanelOpen
    readonly property var jobs: JSON.parse(HermesApp.jobs || "[]")

    Settings {
        id: settings
        category: "chat"
        property bool jobsPanelOpen: true
    }

    // The list goes stale as jobs run; poll while the panel is showing.
    Timer {
        interval: 20000
        repeat: true
        running: panel.visible && HermesApp.connected
        onTriggered: HermesApp.refreshJobs()
    }
    onVisibleChanged: if (visible) HermesApp.refreshJobs()

    // ---- formatting ----

    function relative(iso) {
        const t = iso ? new Date(iso).getTime() : NaN
        if (!isFinite(t))
            return ""
        const seconds = Math.round((t - Date.now()) / 1000)
        const abs = Math.abs(seconds)
        const span = abs < 60 ? "less than a minute"
                   : abs < 3600 ? Math.round(abs / 60) + " min"
                   : abs < 86400 ? Math.round(abs / 3600) + " h"
                   : Math.round(abs / 86400) + " d"
        return seconds >= 0 ? "in " + span : span + " ago"
    }

    function absolute(iso) {
        const d = iso ? new Date(iso) : null
        return d && isFinite(d.getTime()) ? d.toLocaleString(Qt.locale(), Locale.ShortFormat) : ""
    }

    function pad(n) { return (n < 10 ? "0" : "") + n }

    // Hermes schedules are intervals ("every 60m", "once in 2h") or cron
    // expressions; spell out the common shapes and pass the rest through.
    function scheduleLabel(s) {
        let m = /^every (\d+)m$/.exec(s)
        if (m) {
            const min = Number(m[1])
            if (min % 1440 === 0) return min === 1440 ? "Daily" : "Every " + (min / 1440) + " days"
            if (min % 60 === 0) return min === 60 ? "Hourly" : "Every " + (min / 60) + " hours"
            return "Every " + min + " minutes"
        }
        m = /^once in (.+)$/.exec(s) || /^(\d+[mhd])$/.exec(s)
        if (m) return "Once, in " + m[1]
        m = /^(\d+) (\d+) \* \* (\*|[0-6](?:[,-][0-6])*)$/.exec(s)
        if (m) {
            const time = pad(Number(m[2])) + ":" + pad(Number(m[1]))
            if (m[3] === "*") return "Daily at " + time
            if (m[3] === "1-5") return "Weekdays at " + time
            const names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
            const days = m[3].split(",").map(d => d.includes("-") ? d.split("-").map(x => names[Number(x)]).join("–") : names[Number(d)])
            return days.join(", ") + " at " + time
        }
        m = /^(\d+) \* \* \* \*$/.exec(s)
        if (m) return "Hourly at :" + pad(Number(m[1]))
        return s
    }

    function statusText(status) {
        return ({ ok: "Succeeded", error: "Failed", delivery_failed: "Ran, but delivery failed",
                  blocked_config: "Blocked by configuration" })[status] || status
    }

    Rectangle { width: 1; height: parent.height; color: Theme.border }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 1
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            Layout.leftMargin: 16
            Layout.rightMargin: 10
            spacing: 6

            Label {
                text: "Scheduled jobs"
                color: Theme.foreground
                font.pixelSize: 14
                font.weight: Font.DemiBold
            }
            BusyIndicator {
                running: HermesApp.jobsLoading
                visible: running
                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
            }
            Item { Layout.fillWidth: true }
            Repeater {
                model: [
                    { glyph: "↻", tip: "Refresh", act: () => HermesApp.refreshJobs() },
                    { glyph: "✕", tip: "Hide", act: () => panel.open = false }
                ]
                delegate: Rectangle {
                    required property var modelData
                    width: 28
                    height: 28
                    radius: Theme.radius
                    color: btnHover.hovered ? Theme.raised : "transparent"
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

        Label {
            visible: HermesApp.jobsError.length > 0
            Layout.fillWidth: true
            Layout.margins: 16
            text: "Couldn't load jobs: " + HermesApp.jobsError
            color: Theme.danger
            font.pixelSize: 13
            wrapMode: Text.Wrap
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: panel.jobs
            spacing: 8
            topMargin: 12
            bottomMargin: 12
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

            delegate: Item {
                id: job
                required property var modelData
                property bool expanded: false
                readonly property bool failing: modelData.error.length > 0
                                                || ["error", "delivery_failed", "blocked_config"].includes(modelData.lastStatus)
                width: ListView.view.width
                height: card.height

                Rectangle {
                    id: card
                    x: 12
                    width: parent.width - 24
                    height: body.implicitHeight + 24
                    radius: Theme.radius
                    color: cardHover.hovered || job.expanded ? Theme.raised : Qt.alpha(Theme.raised, 0.45)
                    border.width: 1
                    border.color: job.expanded ? Theme.border : "transparent"
                    opacity: job.modelData.paused ? 0.6 : 1

                    ColumnLayout {
                        id: body
                        x: 12
                        y: 12
                        width: parent.width - 24
                        spacing: 4

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8
                            Rectangle {
                                width: 8
                                height: 8
                                radius: 4
                                Layout.alignment: Qt.AlignTop
                                Layout.topMargin: 6
                                color: job.modelData.paused ? Theme.muted : (job.failing ? Theme.danger : "#3fb950")
                            }
                            Label {
                                Layout.fillWidth: true
                                text: job.modelData.title || "Untitled job"
                                color: Theme.foreground
                                font.pixelSize: 14
                                font.weight: Font.Medium
                                wrapMode: Text.Wrap
                                maximumLineCount: job.expanded ? 6 : 2
                                elide: Text.ElideRight
                            }
                        }

                        Label {
                            Layout.fillWidth: true
                            Layout.leftMargin: 16
                            text: panel.scheduleLabel(job.modelData.schedule)
                            color: Theme.muted
                            font.pixelSize: 12
                            elide: Text.ElideRight
                        }
                        Label {
                            Layout.fillWidth: true
                            Layout.leftMargin: 16
                            text: job.modelData.paused ? "Paused"
                                  : (job.modelData.nextRunAt ? "Next " + panel.relative(job.modelData.nextRunAt) : "Not scheduled")
                            color: job.modelData.paused ? Theme.muted : Theme.accent
                            font.pixelSize: 12
                        }

                        // Details.
                        GridLayout {
                            visible: job.expanded
                            Layout.fillWidth: true
                            Layout.leftMargin: 16
                            Layout.topMargin: 8
                            columns: 2
                            columnSpacing: 10
                            rowSpacing: 4

                            Repeater {
                                model: [
                                    ["Next run", panel.absolute(job.modelData.nextRunAt)],
                                    ["Last run", job.modelData.lastRunAt ? panel.relative(job.modelData.lastRunAt) + " · " + panel.absolute(job.modelData.lastRunAt) : "Never"],
                                    ["Result", panel.statusText(job.modelData.lastStatus)],
                                    ["Delivers to", job.modelData.deliver],
                                    ["Schedule", job.modelData.schedule],
                                ].filter(r => r[1] && r[1].length > 0)
                                delegate: Item {
                                    required property var modelData
                                    Layout.columnSpan: 2
                                    Layout.fillWidth: true
                                    implicitHeight: detailRow.implicitHeight
                                    RowLayout {
                                        id: detailRow
                                        width: parent.width
                                        Label {
                                            text: modelData[0]
                                            color: Theme.muted
                                            font.pixelSize: 12
                                            Layout.preferredWidth: 76
                                            Layout.alignment: Qt.AlignTop
                                        }
                                        Label {
                                            Layout.fillWidth: true
                                            text: modelData[1]
                                            color: Theme.foreground
                                            font.pixelSize: 12
                                            wrapMode: Text.Wrap
                                        }
                                    }
                                }
                            }
                        }
                        Label {
                            visible: job.expanded && job.modelData.error.length > 0
                            Layout.fillWidth: true
                            Layout.leftMargin: 16
                            Layout.topMargin: 4
                            text: job.modelData.error
                            color: Theme.danger
                            font.pixelSize: 12
                            wrapMode: Text.Wrap
                        }
                        Label {
                            visible: job.expanded && job.modelData.prompt.length > 0
                            Layout.fillWidth: true
                            Layout.leftMargin: 16
                            Layout.topMargin: 6
                            text: job.modelData.prompt
                            color: Theme.muted
                            font.pixelSize: 12
                            font.italic: true
                            wrapMode: Text.Wrap
                        }
                    }

                    HoverHandler { id: cardHover; cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: job.expanded = !job.expanded }
                }
            }

            ColumnLayout {
                anchors.centerIn: parent
                width: parent.width - 48
                visible: list.count === 0 && !HermesApp.jobsLoading && HermesApp.jobsError.length === 0
                spacing: 6
                Label {
                    Layout.alignment: Qt.AlignHCenter
                    text: "No scheduled jobs"
                    color: Theme.foreground
                    font.pixelSize: 14
                }
                Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    text: "Ask the bot to do something on a schedule, and it will show up here."
                    color: Theme.muted
                    font.pixelSize: 12
                    wrapMode: Text.Wrap
                }
            }
        }
    }
}
