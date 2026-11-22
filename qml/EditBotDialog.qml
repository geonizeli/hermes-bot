import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import HermesBot

// Edit Bot: the official desktop's Edit Profile dialog without its Advanced
// section. A preview, the avatar picker (Bot / Generate / Upload / Pet), the
// title and the description. The logic lives in Rust (backend/botedit.rs).
Dialog {
    id: dialog
    modal: true
    closePolicy: busy ? Popup.NoAutoClose : Popup.CloseOnEscape
    padding: 20
    width: Math.min(440, parent ? parent.width - 40 : 440)
    height: Math.min(implicitHeight, parent ? parent.height - 40 : implicitHeight)

    enter: Transition {
        NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 160; easing.type: Easing.OutCubic }
        NumberAnimation { property: "scale"; from: 0.96; to: 1; duration: 160; easing.type: Easing.OutCubic }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; from: 1; to: 0; duration: 120 }
        NumberAnimation { property: "scale"; from: 1; to: 0.97; duration: 120 }
    }

    // What the dialog was opened for.
    property string botName: ""
    property string displayName: ""
    property var openedLook: ({})
    property var shapes: []
    property var kinds: []
    property var swatches: []

    // The look being edited.
    property string shape: ""
    property string pickedColor: ""
    property string image: ""
    property alias titleText: titleField.text
    property alias descriptionText: descriptionField.text

    property string tab: "bot"
    property bool busy: false
    property string errorText: ""
    // null = not probed yet.
    property var canGenerate: null
    property bool generating: false

    readonly property real dpr: Screen.devicePixelRatio
    readonly property var blob: JSON.parse(HermesApp.blobParts(shape))

    function face(s, size) {
        return HermesApp.faceUrl(s, pickedColor, botName, Math.round(size * dpr))
    }

    function open_(bot) {
        const info = JSON.parse(HermesApp.editBotOpen(bot.name))
        botName = info.name
        displayName = bot.displayName
        openedLook = info.look
        shapes = info.shapes
        kinds = info.kinds
        swatches = info.swatches
        shape = info.look.shape
        pickedColor = info.look.color
        image = info.look.image
        titleText = info.look.title
        descriptionText = info.look.description
        tab = "bot"
        busy = false
        errorText = ""
        generating = false
        describeField.text = ""
        petTab.reset()
        open()
    }

    function current() {
        return { shape: shape, color: pickedColor, image: image, title: titleText, description: descriptionText }
    }

    function save() {
        if (busy)
            return
        busy = true
        errorText = ""
        HermesApp.editSave(botName, JSON.stringify(openedLook), JSON.stringify(current()))
    }

    function pickShape(s) {
        image = ""
        shape = s
    }

    Connections {
        target: HermesApp
        enabled: dialog.visible
        function onEditSaved(ok, message) {
            dialog.busy = false
            if (ok)
                dialog.close()
            else
                dialog.errorText = message
        }
        function onEditImageReady(url, error) {
            dialog.generating = false
            if (url.length > 0)
                dialog.image = url
            else if (error.length > 0)
                dialog.errorText = error
        }
        function onEditGenerateAvailable(available) {
            dialog.canGenerate = available
        }
    }

    header: ColumnLayout {
        spacing: 4
        Label {
            Layout.fillWidth: true
            Layout.topMargin: 20
            Layout.leftMargin: 20
            Layout.rightMargin: 20
            text: "Edit profile"
            color: Theme.foreground
            font.pixelSize: 18
            font.weight: Font.DemiBold
        }
        Label {
            Layout.fillWidth: true
            Layout.leftMargin: 20
            Layout.rightMargin: 20
            text: "Appearance and role for " + dialog.displayName + " (" + dialog.botName + ")."
            color: Theme.muted
            font.pixelSize: 13
            wrapMode: Text.Wrap
        }
    }

    footer: ColumnLayout {
        spacing: 8
        Label {
            Layout.fillWidth: true
            Layout.leftMargin: 20
            Layout.rightMargin: 20
            visible: dialog.errorText.length > 0
            text: dialog.errorText
            color: Theme.danger
            wrapMode: Text.Wrap
            font.pixelSize: 13
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 20
            Layout.rightMargin: 20
            Layout.bottomMargin: 20
            spacing: 8
            Item { Layout.fillWidth: true }
            Button {
                text: "Cancel"
                flat: true
                enabled: !dialog.busy
                onClicked: dialog.close()
            }
            Button {
                text: dialog.busy ? "Saving…" : "Save"
                highlighted: true
                enabled: !dialog.busy
                onClicked: dialog.save()
            }
        }
    }

    contentItem: ScrollView {
        id: scroll
        contentWidth: availableWidth
        clip: true
        implicitHeight: body.implicitHeight

        ColumnLayout {
            id: body
            width: scroll.availableWidth
            spacing: 14

            // Preview.
            Item {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: 64
                Layout.preferredHeight: 64
                Layout.topMargin: 4
                Image {
                    anchors.fill: parent
                    visible: dialog.image.length === 0
                    source: dialog.image.length === 0 && dialog.botName.length > 0 ? dialog.face(dialog.shape, 64) : ""
                    sourceSize: Qt.size(64 * dialog.dpr, 64 * dialog.dpr)
                    smooth: true
                }
                RoundedImage {
                    anchors.fill: parent
                    visible: dialog.image.length > 0
                    source: dialog.image
                }
            }

            // Bot / Generate / Upload / Pet.
            Rectangle {
                Layout.alignment: Qt.AlignHCenter
                implicitWidth: tabs.implicitWidth + 6
                implicitHeight: tabs.implicitHeight + 6
                radius: Theme.radius
                color: Theme.surface
                border.width: 1
                border.color: Theme.border
                Row {
                    id: tabs
                    anchors.centerIn: parent
                    spacing: 2
                    Repeater {
                        model: [
                            { id: "bot", label: "Bot" },
                            { id: "generate", label: "Generate" },
                            { id: "upload", label: "Upload" },
                            { id: "pet", label: "Pet" }
                        ]
                        delegate: Rectangle {
                            id: tabButton
                            required property var modelData
                            readonly property bool on: dialog.tab === modelData.id
                            width: tabLabel.implicitWidth + 24
                            height: 28
                            radius: Theme.radius
                            color: on ? Theme.raised : (tabHover.hovered ? Qt.alpha(Theme.raised, 0.5) : "transparent")
                            Label {
                                id: tabLabel
                                anchors.centerIn: parent
                                text: tabButton.modelData.label
                                color: tabButton.on ? Theme.foreground : Theme.muted
                                font.pixelSize: 13
                                font.weight: tabButton.on ? Font.DemiBold : Font.Normal
                            }
                            HoverHandler { id: tabHover; cursorShape: Qt.PointingHandCursor }
                            TapHandler {
                                onTapped: {
                                    dialog.tab = tabButton.modelData.id
                                    // A stale "unavailable" is re-checked on every visit.
                                    if (dialog.tab === "generate" && dialog.canGenerate !== true) {
                                        dialog.canGenerate = null
                                        HermesApp.editProbeGenerate()
                                    }
                                    if (dialog.tab === "pet")
                                        petTab.load()
                                }
                            }
                        }
                    }
                }
            }

            Button {
                Layout.alignment: Qt.AlignHCenter
                visible: dialog.image.length > 0 && dialog.tab !== "generate"
                text: "Remove image — use shape"
                flat: true
                onClicked: dialog.image = ""
            }

            // ── Bot: blobatar controls ──
            ColumnLayout {
                Layout.fillWidth: true
                visible: dialog.tab === "bot" && dialog.blob.blob
                spacing: 10

                Grid {
                    Layout.alignment: Qt.AlignHCenter
                    columns: 4
                    spacing: 6
                    Repeater {
                        model: [""].concat(dialog.kinds)
                        delegate: FaceTile {
                            required property string modelData
                            tip: modelData.length > 0 ? modelData : "Auto — the name decides"
                            selected: modelData === dialog.blob.kind && dialog.image.length === 0
                            source: modelData.length > 0 ? dialog.face(HermesApp.blobShape(dialog.blob.seed, modelData), 32) : ""
                            label: modelData.length > 0 ? "" : "Auto"
                            onClicked: dialog.pickShape(HermesApp.blobShape(dialog.blob.seed, modelData))
                        }
                    }
                }
                Row {
                    Layout.alignment: Qt.AlignHCenter
                    spacing: 4
                    Button {
                        text: "Randomize"
                        flat: true
                        onClicked: dialog.pickShape(HermesApp.blobShape(HermesApp.randomSeed(), dialog.blob.kind))
                    }
                    Button {
                        readonly property bool locked: dialog.blob.seed.length > 0
                        text: locked ? "Unlock" : "Lock face"
                        flat: true
                        onClicked: dialog.shape = HermesApp.blobShape(locked ? "" : dialog.botName, dialog.blob.kind)
                    }
                }
                Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    text: dialog.blob.seed.length > 0 ? "Face locked — renaming won’t change it." : "Face follows the name."
                    color: Theme.muted
                    font.pixelSize: 11
                }
                Button {
                    Layout.alignment: Qt.AlignHCenter
                    text: "Classic shapes"
                    flat: true
                    onClicked: dialog.shape = HermesApp.defaultShape(dialog.botName)
                }
            }

            // ── Bot: classic shapes and colors ──
            ColumnLayout {
                Layout.fillWidth: true
                visible: dialog.tab === "bot" && !dialog.blob.blob
                spacing: 12

                Grid {
                    Layout.alignment: Qt.AlignHCenter
                    columns: 4
                    spacing: 6
                    Repeater {
                        model: ["blobatar"].concat(dialog.shapes)
                        delegate: FaceTile {
                            required property string modelData
                            tip: modelData === "blobatar" ? "Blob face — drawn from the bot’s name" : modelData
                            selected: modelData === dialog.shape && dialog.image.length === 0
                            source: dialog.face(modelData, 32)
                            onClicked: dialog.pickShape(modelData)
                        }
                    }
                }

                ColumnLayout {
                    Layout.alignment: Qt.AlignHCenter
                    spacing: 6
                    Grid {
                        Layout.alignment: Qt.AlignHCenter
                        columns: 6
                        spacing: 8
                        Repeater {
                            model: dialog.swatches
                            delegate: Rectangle {
                                id: swatch
                                required property string modelData
                                readonly property bool on: modelData === dialog.pickedColor
                                width: 22
                                height: 22
                                radius: 11
                                color: HermesApp.faceColor(modelData, dialog.botName)
                                border.width: on ? 2 : 0
                                border.color: Theme.foreground
                                scale: swatchHover.hovered ? 1.1 : 1
                                Behavior on scale { NumberAnimation { duration: 100 } }
                                HoverHandler { id: swatchHover; cursorShape: Qt.PointingHandCursor }
                                TapHandler { onTapped: dialog.pickedColor = swatch.modelData }
                            }
                        }
                    }
                    Button {
                        Layout.fillWidth: true
                        text: "Match the name"
                        flat: true
                        onClicked: dialog.pickedColor = ""
                    }
                }
            }

            // ── Generate ──
            ColumnLayout {
                Layout.fillWidth: true
                visible: dialog.tab === "generate"
                spacing: 8

                Label {
                    Layout.fillWidth: true
                    visible: dialog.canGenerate !== true
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    text: dialog.canGenerate === false
                          ? "No image model available. If you just enabled one (or updated Hermes), restart the gateway."
                          : "Checking image backend…"
                    color: Theme.muted
                    font.pixelSize: 12
                }
                TextArea {
                    id: describeField
                    Layout.fillWidth: true
                    Layout.minimumHeight: 64
                    visible: dialog.canGenerate === true
                    placeholderText: "Describe your avatar…"
                    wrapMode: TextEdit.Wrap
                }
                Button {
                    Layout.fillWidth: true
                    visible: dialog.canGenerate === true
                    enabled: !dialog.generating
                    text: dialog.generating ? "Generating…" : "Generate"
                    onClicked: {
                        dialog.generating = true
                        dialog.errorText = ""
                        HermesApp.editGenerate(describeField.text, dialog.botName, dialog.titleText, dialog.descriptionText)
                    }
                }
                Label {
                    Layout.fillWidth: true
                    visible: dialog.canGenerate === true && describeField.text.trim().length === 0
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    text: "Leave blank to generate from the bot’s name and description."
                    color: Theme.muted
                    font.pixelSize: 11
                }
            }

            // ── Upload ──
            Button {
                Layout.fillWidth: true
                visible: dialog.tab === "upload"
                text: "Choose an image…"
                onClicked: HermesApp.editPickImage()
            }

            // ── Pet ──
            PetPicker {
                id: petTab
                Layout.fillWidth: true
                visible: dialog.tab === "pet"
                image: dialog.image
                onPicked: url => dialog.image = url
                onRemoved: dialog.image = ""
            }

            // Title and description.
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 4
                Label { text: "Title"; color: Theme.muted; font.pixelSize: 12 }
                TextField {
                    id: titleField
                    Layout.fillWidth: true
                    placeholderText: dialog.displayName
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 4
                Label { text: "Description"; color: Theme.muted; font.pixelSize: 12 }
                TextArea {
                    id: descriptionField
                    Layout.fillWidth: true
                    Layout.minimumHeight: 64
                    placeholderText: "What should this bot help with?"
                    wrapMode: TextEdit.Wrap
                }
            }
        }
    }

    // One tile of the shape grid.
    component FaceTile: Rectangle {
        id: tile
        property string source: ""
        property string label: ""
        property string tip: ""
        property bool selected: false
        signal clicked()
        width: 44
        height: 44
        radius: Theme.radius
        color: tileHover.hovered ? Theme.raised : "transparent"
        border.width: selected ? 1 : 0
        border.color: Theme.accent
        Image {
            anchors.centerIn: parent
            width: 32
            height: 32
            visible: tile.source.length > 0
            source: tile.source
            sourceSize: Qt.size(32 * dialog.dpr, 32 * dialog.dpr)
            smooth: true
        }
        Label {
            anchors.centerIn: parent
            visible: tile.label.length > 0
            text: tile.label
            color: Theme.muted
            font.pixelSize: 10
        }
        HoverHandler { id: tileHover; cursorShape: Qt.PointingHandCursor }
        TapHandler { onTapped: tile.clicked() }
        ToolTip.visible: tileHover.hovered && tip.length > 0
        ToolTip.delay: 400
        ToolTip.text: tip
    }

    // A picture, cropped to fill its square.
    component RoundedImage: Item {
        property alias source: picture.source
        Image {
            id: picture
            anchors.fill: parent
            fillMode: Image.PreserveAspectCrop
            sourceSize: Qt.size(width * dialog.dpr, height * dialog.dpr)
            smooth: true
            mipmap: true
        }
    }
}
