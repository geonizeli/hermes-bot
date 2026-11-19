import QtQuick
import QtQuick.Controls
import HermesBot

// A bot's initial in a circle, tinted from its name so each bot keeps the
// same color.
Rectangle {
    id: avatar
    property string name: ""
    property int size: 36

    width: size
    height: size
    radius: size / 2

    function hue(text) {
        let h = 0
        for (let i = 0; i < text.length; i++)
            h = (h * 31 + text.charCodeAt(i)) % 360
        return h / 360
    }
    color: Qt.hsla(hue(name), 0.45, 0.42, 1)

    Label {
        anchors.centerIn: parent
        text: avatar.name.length > 0 ? avatar.name.charAt(0).toUpperCase() : "?"
        color: "white"
        font.pixelSize: Math.round(avatar.size * 0.44)
        font.weight: Font.DemiBold
    }
}
