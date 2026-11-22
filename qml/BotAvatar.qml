import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import HermesBot

// A bot's avatar from the server, cropped to a circle. Until it loads, or
// when the bot has none, its initial in a circle tinted from its name so each
// bot keeps the same color.
Rectangle {
    id: avatar
    property string name: ""
    property string source: ""
    property int size: 36
    readonly property bool hasImage: picture.status === Image.Ready

    width: size
    height: size
    radius: size / 2

    function hue(text) {
        let h = 0
        for (let i = 0; i < text.length; i++)
            h = (h * 31 + text.charCodeAt(i)) % 360
        return h / 360
    }
    color: hasImage ? "transparent" : Qt.hsla(hue(name), 0.45, 0.42, 1)

    Label {
        anchors.centerIn: parent
        visible: !avatar.hasImage
        text: avatar.name.length > 0 ? avatar.name.charAt(0).toUpperCase() : "?"
        color: "white"
        font.pixelSize: Math.round(avatar.size * 0.44)
        font.weight: Font.DemiBold
    }

    Image {
        id: picture
        anchors.fill: parent
        source: avatar.source
        visible: false
        asynchronous: true
        fillMode: Image.PreserveAspectCrop
        sourceSize.width: avatar.size * 2
        sourceSize.height: avatar.size * 2
        smooth: true
        mipmap: true
    }

    Rectangle {
        id: mask
        anchors.fill: parent
        radius: width / 2
        visible: false
        layer.enabled: true
    }

    MultiEffect {
        anchors.fill: parent
        visible: avatar.hasImage
        source: picture
        maskEnabled: true
        maskSource: mask
        maskThresholdMin: 0.5
        maskSpreadAtMin: 1.0
    }
}
