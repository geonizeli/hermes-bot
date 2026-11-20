import QtQuick
import QtQuick.Controls
import HermesBot

// An image that lives on the Hermes server (a path) or on this machine (a
// file:// URL). Server images are fetched once and cached by the backend.
Item {
    id: root
    property string path: ""      // server-side path
    property string url: ""       // local file:// URL, when known
    property int maxWidth: 320
    property int maxHeight: 240
    readonly property string source: url.length > 0 ? url : fetched
    property string fetched: ""
    signal clicked()

    implicitWidth: image.status === Image.Ready ? image.paintedWidth : 160
    implicitHeight: image.status === Image.Ready ? image.paintedHeight : 100

    Component.onCompleted: if (url.length === 0 && path.length > 0) HermesApp.requestMedia(path)

    Connections {
        target: HermesApp
        enabled: root.url.length === 0 && root.fetched.length === 0
        function onMediaReady(path, url) {
            if (path === root.path)
                root.fetched = url
        }
    }

    Rectangle {
        anchors.fill: parent
        visible: image.status !== Image.Ready
        color: Theme.surface
        radius: Theme.radius
        border.width: 1
        border.color: Theme.border
        BusyIndicator {
            anchors.centerIn: parent
            running: image.status !== Image.Error && root.source.length === 0 || image.status === Image.Loading
            width: 24
            height: 24
        }
    }

    Image {
        id: image
        source: root.source
        asynchronous: true
        fillMode: Image.PreserveAspectFit
        width: Math.min(root.maxWidth, sourceSize.width)
        height: Math.min(root.maxHeight, sourceSize.height)
        sourceSize.width: 1600
        smooth: true
        mipmap: true
    }

    HoverHandler { cursorShape: Qt.PointingHandCursor }
    TapHandler { onTapped: root.clicked() }
}
