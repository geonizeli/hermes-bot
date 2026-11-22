import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// The Pet tab: browse the petdex gallery and use a pet's first frame as the
// bot's picture. The gallery has thousands of pets, so tiles are shown 24 at
// a time and grow as the list scrolls; thumbnails are cropped by the server.
ColumnLayout {
    id: picker
    spacing: 8

    property string image: ""
    signal picked(string url)
    signal removed()

    property var pets: []
    property bool loading: false
    property bool loaded: false
    property string error: ""
    property string selectedSlug: ""
    property int limit: 24
    // slug -> file URL ("" while loading).
    property var thumbs: ({})

    readonly property var ranked: {
        const q = search.text.trim().toLowerCase()
        return q.length === 0 ? pets
             : pets.filter(p => p.name.toLowerCase().indexOf(q) >= 0 || p.slug.indexOf(q) >= 0)
    }
    readonly property var visiblePets: ranked.slice(0, limit)

    function reset() {
        selectedSlug = ""
        search.text = ""
        limit = 24
        error = ""
    }

    function load() {
        if (loaded || loading)
            return
        loading = true
        HermesApp.editLoadPets()
    }

    function thumb(pet) {
        if (!(pet.slug in thumbs)) {
            thumbs[pet.slug] = ""
            HermesApp.editPetThumb(pet.slug, pet.url)
        }
        return thumbs[pet.slug]
    }

    Connections {
        target: HermesApp
        function onEditPetsLoaded(json, error) {
            picker.loading = false
            picker.error = error
            if (error.length === 0) {
                picker.pets = JSON.parse(json)
                picker.loaded = true
            }
        }
        function onEditPetReady(slug, url) {
            if (url.length === 0) {
                // Never keep a failure: a later visit retries.
                delete picker.thumbs[slug]
                if (slug === picker.selectedSlug) {
                    picker.selectedSlug = ""
                    picker.error = "Could not load that pet — try another."
                }
            } else {
                picker.thumbs[slug] = url
                if (slug === picker.selectedSlug)
                    picker.picked(url)
            }
            picker.thumbs = Object.assign({}, picker.thumbs)
        }
    }

    BusyIndicator {
        Layout.alignment: Qt.AlignHCenter
        visible: picker.loading
        running: visible
    }

    Label {
        Layout.fillWidth: true
        visible: !picker.loading && picker.loaded && picker.pets.length === 0
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: "No pets in the petdex gallery. Run `hermes pets` to explore."
        color: Theme.muted
        font.pixelSize: 12
    }

    Label {
        Layout.fillWidth: true
        visible: picker.error.length > 0
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: picker.error
        color: Theme.danger
        font.pixelSize: 12
    }

    ColumnLayout {
        Layout.fillWidth: true
        visible: picker.loaded && picker.pets.length > 0
        spacing: 8

        Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: "Pick a pet as this bot’s profile picture."
            color: Theme.muted
            font.pixelSize: 11
        }
        TextField {
            id: search
            Layout.fillWidth: true
            placeholderText: "Search " + picker.pets.length + " pets…"
            onTextChanged: picker.limit = 24
        }
        Button {
            Layout.alignment: Qt.AlignHCenter
            visible: picker.image.length > 0 && picker.selectedSlug.length > 0
            text: "Remove — back to shape avatar"
            flat: true
            onClicked: {
                picker.selectedSlug = ""
                picker.removed()
            }
        }
        Label {
            Layout.fillWidth: true
            visible: picker.ranked.length === 0
            horizontalAlignment: Text.AlignHCenter
            text: "No pets match."
            color: Theme.muted
            font.pixelSize: 12
        }

        GridView {
            id: grid
            Layout.fillWidth: true
            Layout.preferredHeight: 220
            visible: picker.ranked.length > 0
            clip: true
            cellWidth: Math.floor(width / 3)
            cellHeight: 72
            model: picker.visiblePets
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar {}
            onContentYChanged: {
                if (contentY + height >= contentHeight - 120 && picker.limit < picker.ranked.length)
                    picker.limit = Math.min(picker.limit + 24, picker.ranked.length)
            }

            delegate: Item {
                id: cell
                required property var modelData
                readonly property bool selected: picker.selectedSlug === modelData.slug
                width: grid.cellWidth
                height: grid.cellHeight

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 2
                    radius: Theme.radius
                    color: cellHover.hovered ? Theme.raised : "transparent"
                    border.width: cell.selected ? 1 : 0
                    border.color: Theme.accent
                }
                Column {
                    anchors.centerIn: parent
                    width: parent.width - 8
                    spacing: 4
                    Item {
                        anchors.horizontalCenter: parent.horizontalCenter
                        width: 40
                        height: 40
                        Rectangle {
                            anchors.fill: parent
                            radius: Theme.radius
                            color: Theme.surface
                            visible: art.status !== Image.Ready
                        }
                        Image {
                            id: art
                            anchors.fill: parent
                            source: picker.thumbs[cell.modelData.slug] || picker.thumb(cell.modelData)
                            fillMode: Image.PreserveAspectFit
                            smooth: false
                        }
                    }
                    Label {
                        width: parent.width
                        horizontalAlignment: Text.AlignHCenter
                        text: cell.modelData.name
                        color: Theme.muted
                        font.pixelSize: 10
                        elide: Text.ElideRight
                    }
                }
                HoverHandler { id: cellHover; cursorShape: Qt.PointingHandCursor }
                TapHandler {
                    onTapped: {
                        picker.error = ""
                        picker.selectedSlug = cell.modelData.slug
                        const url = picker.thumbs[cell.modelData.slug]
                        if (url)
                            picker.picked(url)
                        else
                            picker.thumb(cell.modelData)
                    }
                }
            }
        }

        Label {
            Layout.fillWidth: true
            visible: picker.limit < picker.ranked.length
            horizontalAlignment: Text.AlignHCenter
            text: "Scroll for more (" + picker.limit + " of " + picker.ranked.length + ")"
            color: Theme.muted
            font.pixelSize: 10
        }
    }
}
