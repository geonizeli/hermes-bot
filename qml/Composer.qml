import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// The message box: grows with its text, Enter sends, Shift+Enter breaks the
// line, and the round button turns into Stop while the bot answers. Below the
// text: the chat's model, reasoning effort and fast mode.
Rectangle {
    id: composer
    property string botName: "the bot"
    property var bot: null
    // Editing your latest message: sending replaces it and what followed.
    property bool editing: false
    signal editSubmitted(string text)

    function startEdit(text) {
        editing = true
        input.text = text
        input.forceActiveFocus()
        input.cursorPosition = input.length
    }
    function cancelEdit() {
        editing = false
        input.text = ""
    }

    // ---- session state ----
    readonly property var info: {
        try { return HermesApp.sessionInfo ? JSON.parse(HermesApp.sessionInfo) : ({}) } catch (e) { return ({}) }
    }
    readonly property var providers: {
        try { return JSON.parse(HermesApp.models || "[]") } catch (e) { return [] }
    }
    readonly property string currentModel: info.model || (bot ? bot.model : "")
    readonly property string currentProvider: info.provider || ""
    // Capabilities of the current model, from whichever provider lists it.
    readonly property var caps: {
        for (const p of providers) {
            if (currentProvider && p.slug !== currentProvider && p.name !== currentProvider)
                continue
            const c = p.capabilities ? p.capabilities[currentModel] : null
            if (c) return c
        }
        return ({})
    }
    readonly property bool showsEffort: caps.reasoning !== false
    readonly property bool showsFast: !!(caps.fast || caps.ultrafast) || !!info.fast
    readonly property string effort: info.reasoning_effort || "medium"

    readonly property var pending: {
        try { return JSON.parse(HermesApp.attachments || "[]") } catch (e) { return [] }
    }
    implicitHeight: Math.min(Math.max(input.implicitHeight, 24), 220) + 28 + controls.implicitHeight + 6
                    + (pending.length > 0 ? chips.implicitHeight + 6 : 0)
                    + (editing ? editBanner.implicitHeight + 6 : 0)
    radius: Math.max(Theme.radius, 0)
    color: Theme.raised
    border.width: 1
    border.color: input.activeFocus ? Qt.alpha(Theme.accent, 0.6) : Theme.border
    Behavior on border.color { ColorAnimation { duration: 160 } }
    Behavior on implicitHeight { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

    function focusInput() { input.forceActiveFocus() }

    function send() {
        const text = input.text
        if (HermesApp.streaming) {
            // Typing while the bot works steers it; an empty send stops it.
            if (text.trim().length > 0) {
                HermesApp.steer(text)
                input.text = ""
            } else {
                HermesApp.stop()
            }
            return
        }
        if (editing) {
            if (text.trim().length === 0) return
            editing = false
            input.text = ""
            editSubmitted(text)
            return
        }
        if (text.trim().length === 0 && pending.length === 0)
            return
        HermesApp.send(text)
        input.text = ""
    }

    // A small rounded control in the bottom row.
    component Chip: Rectangle {
        id: chip
        property string text
        property bool active: false
        signal clicked()
        implicitWidth: chipLabel.implicitWidth + 20
        implicitHeight: 28
        radius: Theme.radius
        color: active ? Qt.alpha(Theme.accent, 0.18) : (chipHover.hovered ? Theme.surface : Qt.alpha(Theme.surface, 0))
        border.width: 1
        border.color: active ? Qt.alpha(Theme.accent, 0.6) : Theme.border
        opacity: enabled ? 1 : 0.5
        Behavior on color { ColorAnimation { duration: 120 } }
        Label {
            id: chipLabel
            anchors.centerIn: parent
            text: chip.text
            color: chip.active ? Theme.foreground : Theme.muted
            font.pixelSize: 12
            elide: Text.ElideRight
            width: Math.min(implicitWidth, 260)
        }
        HoverHandler { id: chipHover; cursorShape: Qt.PointingHandCursor }
        TapHandler { enabled: chip.enabled; onTapped: chip.clicked() }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 16
        anchors.rightMargin: 10
        anchors.topMargin: 10
        anchors.bottomMargin: 10
        spacing: 6

        RowLayout {
            id: editBanner
            visible: composer.editing
            Layout.fillWidth: true
            Label {
                Layout.fillWidth: true
                text: "Editing your message. Sending replaces it and the answers after it."
                color: Theme.accent
                font.pixelSize: 12
                elide: Text.ElideRight
            }
            Label {
                text: "Cancel"
                color: cancelHover.hovered ? Theme.foreground : Theme.muted
                font.pixelSize: 12
                HoverHandler { id: cancelHover; cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: composer.cancelEdit() }
            }
        }

        // Files waiting to be sent.
        Flow {
            id: chips
            Layout.fillWidth: true
            visible: composer.pending.length > 0
            spacing: 6
            Repeater {
                model: composer.pending
                delegate: Rectangle {
                    id: pendingChip
                    required property var modelData
                    readonly property bool isImage: modelData.kind === "image"
                    width: isImage ? 64 : fileRow.implicitWidth + 16
                    height: isImage ? 64 : 34
                    radius: Theme.radius
                    color: Theme.surface
                    border.width: 1
                    border.color: Theme.border
                    clip: true
                    Image {
                        anchors.fill: parent
                        visible: pendingChip.isImage
                        source: pendingChip.isImage ? pendingChip.modelData.url : ""
                        fillMode: Image.PreserveAspectCrop
                        sourceSize.width: 128
                        asynchronous: true
                    }
                    Row {
                        id: fileRow
                        visible: !pendingChip.isImage
                        anchors.verticalCenter: parent.verticalCenter
                        x: 8
                        spacing: 6
                        Label { text: "📄"; font.pixelSize: 13 }
                        Label {
                            text: pendingChip.modelData.name
                            color: Theme.foreground
                            font.pixelSize: 12
                            elide: Text.ElideMiddle
                            width: Math.min(implicitWidth, 200)
                        }
                        Item { width: 14; height: 1 }
                    }
                    // Remove.
                    Rectangle {
                        anchors.top: parent.top
                        anchors.right: parent.right
                        anchors.margins: 3
                        width: 18
                        height: 18
                        radius: 9
                        color: Qt.alpha("black", removeHover.hovered ? 0.8 : 0.55)
                        Label { anchors.centerIn: parent; text: "✕"; color: "white"; font.pixelSize: 10 }
                        HoverHandler { id: removeHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: HermesApp.removeAttachment(pendingChip.modelData.id) }
                    }
                }
            }
        }

        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true

            TextArea {
                id: input
                placeholderText: !HermesApp.connected ? "Not connected"
                                 : HermesApp.streaming ? "Steer " + composer.botName + "… (empty send stops)"
                                 : "Message " + composer.botName
                placeholderTextColor: Theme.muted
                color: Theme.foreground
                selectionColor: Qt.alpha(Theme.accent, 0.4)
                selectedTextColor: Theme.foreground
                font.pixelSize: 15
                wrapMode: TextEdit.Wrap
                enabled: HermesApp.connected && !HermesApp.chatLoading
                focus: true
                background: null
                padding: 4
                // Images and long texts on the clipboard become attachments.
                Keys.onPressed: event => {
                    if (event.matches(StandardKey.Paste) && HermesApp.pasteClipboard())
                        event.accepted = true
                }
                Keys.onReturnPressed: event => {
                    if (event.modifiers & Qt.ShiftModifier)
                        event.accepted = false
                    else
                        composer.send()
                }
                Keys.onEnterPressed: event => Keys.returnPressed(event)
            }
        }

        RowLayout {
            id: controls
            Layout.fillWidth: true
            spacing: 6
            enabled: HermesApp.connected && !HermesApp.chatLoading

            Chip {
                text: "+"
                implicitWidth: 28
                ToolTip.visible: hovered
                ToolTip.text: "Attach files (or drop / paste them)"
                ToolTip.delay: 600
                readonly property bool hovered: plusHover.hovered
                HoverHandler { id: plusHover }
                onClicked: HermesApp.pickFiles()
            }
            Chip {
                text: (composer.currentModel || "Model") + "  ▾"
                Layout.maximumWidth: 280
                onClicked: { HermesApp.refreshModels(); modelPicker.open() }
            }
            Chip {
                visible: composer.showsEffort
                text: "Effort: " + composer.effort + "  ▾"
                onClicked: effortMenu.popup()
                Menu {
                    id: effortMenu
                    Instantiator {
                        model: (composer.caps.can_disable_reasoning ? ["none"] : [])
                               .concat(["minimal", "low", "medium", "high", "xhigh", "max"])
                        delegate: MenuItem {
                            required property string modelData
                            text: modelData === "none" ? "none (no thinking)" : modelData
                            checkable: true
                            checked: modelData === composer.effort
                            onTriggered: HermesApp.setEffort(modelData)
                        }
                        onObjectAdded: (index, object) => effortMenu.insertItem(index, object)
                        onObjectRemoved: (index, object) => effortMenu.removeItem(object)
                    }
                }
            }
            Chip {
                visible: composer.showsFast
                text: "⚡ Fast"
                active: !!composer.info.fast
                onClicked: HermesApp.setFast(!composer.info.fast)
            }

            Item { Layout.fillWidth: true }

            // Send / Stop.
            Rectangle {
                id: action
                readonly property bool hasText: input.text.trim().length > 0
                readonly property bool canSend: HermesApp.connected && (hasText || composer.pending.length > 0)
                readonly property bool live: HermesApp.streaming || canSend
                // While the bot works: an arrow steers when there's text, a square stops.
                readonly property bool stopping: HermesApp.streaming && !hasText
                width: 36
                height: 36
                radius: 18
                color: live ? Theme.accent : Theme.border
                opacity: actionHover.hovered && live ? 0.85 : 1
                scale: actionTap.pressed && live ? 0.9 : 1
                Behavior on color { ColorAnimation { duration: 160 } }
                Behavior on opacity { NumberAnimation { duration: 120 } }
                Behavior on scale { NumberAnimation { duration: 100; easing.type: Easing.OutCubic } }

                // Arrow up to send, a square to stop.
                Canvas {
                    anchors.centerIn: parent
                    width: 16
                    height: 16
                    visible: !action.stopping
                    property color ink: action.live ? Theme.accentForeground : Theme.muted
                    onInkChanged: requestPaint()
                    onPaint: {
                        const ctx = getContext("2d")
                        ctx.reset()
                        ctx.strokeStyle = ink
                        ctx.lineWidth = 2
                        ctx.lineCap = "round"
                        ctx.lineJoin = "round"
                        ctx.beginPath()
                        ctx.moveTo(8, 14); ctx.lineTo(8, 2)
                        ctx.moveTo(3, 7); ctx.lineTo(8, 2); ctx.lineTo(13, 7)
                        ctx.stroke()
                    }
                }
                Rectangle {
                    anchors.centerIn: parent
                    visible: action.stopping
                    width: 12
                    height: 12
                    radius: 2
                    color: Theme.accentForeground
                }

                HoverHandler { id: actionHover; cursorShape: action.live ? Qt.PointingHandCursor : Qt.ArrowCursor }
                TapHandler {
                    id: actionTap
                    enabled: action.live
                    onTapped: composer.send()
                }
            }
        }
    }

    ModelPicker {
        id: modelPicker
        providers: composer.providers
        currentModel: composer.currentModel
        currentProvider: composer.currentProvider
        x: 0
        y: -height - 8
        width: Math.min(420, composer.width)
    }

    // Expensive models need a yes first.
    Dialog {
        id: confirmDialog
        property string model
        property string provider
        property string message
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        title: "Switch model?"
        width: 420
        standardButtons: Dialog.Yes | Dialog.Cancel
        Label {
            width: parent.width
            text: confirmDialog.message
            wrapMode: Text.Wrap
        }
        onAccepted: HermesApp.setModel(model, provider, true)
    }
    Connections {
        target: HermesApp
        function onModelConfirm(message, model, provider) {
            confirmDialog.message = message
            confirmDialog.model = model
            confirmDialog.provider = provider
            confirmDialog.open()
        }
    }
}
