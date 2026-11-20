import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Qt.labs.qmlmodels
import HermesBot

// A bot answer, rendered by the Rust Markdown renderer into blocks: rich
// text, highlighted code with a copy button, tables, images and alerts.
// Blocks are updated in place while an answer streams, so text stays
// selectable and nothing flickers.
Column {
    id: md
    property string markdown: ""
    spacing: 10

    readonly property string colors: JSON.stringify({
        foreground: Theme.foreground, muted: Theme.muted, accent: Theme.accent,
        surface: Theme.surface, border: Theme.border
    })

    ListModel { id: blocks }

    // Coalesce bursts of streamed deltas into one render.
    Timer {
        id: renderTimer
        interval: 40
        onTriggered: md.render()
    }
    onMarkdownChanged: renderTimer.restart()
    onColorsChanged: renderTimer.restart()
    Component.onCompleted: render()

    function render() {
        let next = []
        try { next = JSON.parse(HermesApp.renderMarkdown(markdown, colors)) } catch (e) { next = [] }
        const keys = ["type", "html", "raw", "lang", "path", "url", "alt", "kind"]
        for (let i = 0; i < next.length; i++) {
            const b = Object.assign({ type: "text", html: "", raw: "", lang: "", path: "", url: "", alt: "", kind: "" }, next[i])
            if (i < blocks.count) {
                const cur = blocks.get(i)
                if (cur.type !== b.type) {
                    blocks.set(i, b)
                } else {
                    for (const k of keys)
                        if (cur[k] !== b[k])
                            blocks.setProperty(i, k, b[k])
                }
            } else {
                blocks.append(b)
            }
        }
        if (blocks.count > next.length)
            blocks.remove(next.length, blocks.count - next.length)
    }

    function copy(text) {
        clipboard.text = text
        clipboard.selectAll()
        clipboard.copy()
    }
    TextEdit { id: clipboard; visible: false }

    component RichText: TextEdit {
        textFormat: TextEdit.RichText
        color: Theme.foreground
        font.pixelSize: 15
        wrapMode: TextEdit.Wrap
        readOnly: true
        selectByMouse: true
        selectionColor: Qt.alpha(Theme.accent, 0.4)
        selectedTextColor: Theme.foreground
        onLinkActivated: link => {
            if (link.startsWith("hermes-media:"))
                HermesApp.requestMedia(link.substring(13))
            else
                Qt.openUrlExternally(link)
        }
        HoverHandler { cursorShape: parent.hoveredLink ? Qt.PointingHandCursor : Qt.IBeamCursor }
    }

    Repeater {
        model: blocks
        delegate: DelegateChooser {
            role: "type"

            DelegateChoice {
                roleValue: "text"
                RichText {
                    required property string html
                    width: md.width
                    text: html
                }
            }

            DelegateChoice {
                roleValue: "code"
                Rectangle {
                    id: codeBlock
                    required property string html
                    required property string raw
                    required property string lang
                    property bool copied: false
                    width: md.width
                    height: codeHeader.height + codeScroll.height
                    radius: Theme.radius
                    color: Theme.surface
                    border.width: 1
                    border.color: Theme.border
                    clip: true

                    Rectangle {
                        id: codeHeader
                        width: parent.width
                        height: 30
                        color: Qt.alpha(Theme.raised, 0.6)
                        Label {
                            anchors.verticalCenter: parent.verticalCenter
                            x: 12
                            text: codeBlock.lang || "code"
                            color: Theme.muted
                            font.pixelSize: 12
                            font.family: "monospace"
                        }
                        Label {
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.right: parent.right
                            anchors.rightMargin: 12
                            text: codeBlock.copied ? "Copied" : "Copy"
                            color: copyHover.hovered ? Theme.foreground : Theme.muted
                            font.pixelSize: 12
                            HoverHandler { id: copyHover; cursorShape: Qt.PointingHandCursor }
                            TapHandler {
                                onTapped: {
                                    md.copy(codeBlock.raw)
                                    codeBlock.copied = true
                                    copiedReset.restart()
                                }
                            }
                            Timer { id: copiedReset; interval: 1500; onTriggered: codeBlock.copied = false }
                        }
                    }

                    // Long lines scroll sideways instead of wrapping.
                    Flickable {
                        id: codeScroll
                        y: codeHeader.height
                        width: parent.width
                        height: Math.min(codeText.implicitHeight + 24, 560)
                        contentWidth: codeText.implicitWidth + 24
                        contentHeight: codeText.implicitHeight + 24
                        clip: true
                        boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.horizontal: ScrollBar { policy: ScrollBar.AsNeeded }
                        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
                        TextEdit {
                            id: codeText
                            x: 12
                            y: 12
                            textFormat: TextEdit.RichText
                            text: codeBlock.html
                            color: Theme.foreground
                            font.family: "monospace"
                            font.pixelSize: 13
                            wrapMode: TextEdit.NoWrap
                            readOnly: true
                            selectByMouse: true
                            selectionColor: Qt.alpha(Theme.accent, 0.4)
                        }
                    }
                }
            }

            DelegateChoice {
                roleValue: "table"
                Flickable {
                    id: tableFlick
                    required property string html
                    width: md.width
                    height: tableText.implicitHeight
                    contentWidth: Math.max(width, tableText.implicitWidth)
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.horizontal: ScrollBar { policy: ScrollBar.AsNeeded }
                    RichText {
                        id: tableText
                        text: tableFlick.html
                        wrapMode: TextEdit.NoWrap
                        font.pixelSize: 14
                    }
                }
            }

            DelegateChoice {
                roleValue: "image"
                Column {
                    required property string path
                    required property string url
                    required property string alt
                    width: md.width
                    spacing: 4
                    RemoteImage {
                        path: parent.path
                        url: parent.url
                        maxWidth: Math.min(md.width, 640)
                        maxHeight: 480
                        onClicked: Qt.openUrlExternally(source)
                    }
                    Label {
                        visible: parent.alt.length > 0
                        text: parent.alt
                        color: Theme.muted
                        font.pixelSize: 12
                    }
                }
            }

            DelegateChoice {
                roleValue: "alert"
                Rectangle {
                    id: alert
                    required property string html
                    required property string kind
                    readonly property color edge: ({ note: "#4493f8", tip: "#3fb950", important: "#ab7df8",
                                                     warning: "#d29922", caution: Theme.danger })[kind] || Theme.accent
                    width: md.width
                    height: alertText.implicitHeight + 36
                    radius: Theme.radius
                    color: Qt.alpha(edge, 0.08)
                    Rectangle { width: 3; height: parent.height; color: alert.edge }
                    Label {
                        x: 16
                        y: 8
                        text: alert.kind.charAt(0).toUpperCase() + alert.kind.slice(1)
                        color: alert.edge
                        font.pixelSize: 12
                        font.weight: Font.DemiBold
                    }
                    RichText {
                        id: alertText
                        x: 16
                        y: 26
                        width: parent.width - 28
                        text: alert.html
                    }
                }
            }

            DelegateChoice {
                roleValue: "rule"
                Rectangle {
                    width: md.width
                    height: 1
                    color: Theme.border
                }
            }
        }
    }
}
