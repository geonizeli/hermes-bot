import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// Add, sign in to, and remove Hermes servers.
Dialog {
    id: dialog
    title: "Servers"
    modal: true
    width: Math.min(560, parent ? parent.width - 40 : 560)
    standardButtons: Dialog.Close

    enter: Transition {
        NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 160; easing.type: Easing.OutCubic }
        NumberAnimation { property: "scale"; from: 0.96; to: 1; duration: 160; easing.type: Easing.OutCubic }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; from: 1; to: 0; duration: 120 }
        NumberAnimation { property: "scale"; from: 1; to: 0.97; duration: 120 }
    }

    property var servers: []
    // The probed server, as returned by HermesApp.probeServer.
    property var info: null
    property string message: ""

    readonly property var provider: info && info.providers.length > 0 ? info.providers[providerBox.currentIndex] : null
    readonly property bool usesPassword: provider ? provider.supports_password : false

    function clearForm() {
        info = null
        message = ""
        urlField.text = ""
        nameField.text = ""
        userField.text = ""
        passwordField.text = ""
    }

    // Re-sign in to a saved server: fill the form and probe it.
    function edit(server) {
        clearForm()
        urlField.text = server.url
        nameField.text = server.name
        userField.text = server.username
        HermesApp.probeServer(server.url)
    }

    onOpened: if (servers.length === 0) urlField.forceActiveFocus()
    onClosed: { passwordField.text = ""; message = "" }

    Connections {
        target: HermesApp
        function onServerProbed(json) {
            dialog.info = json.length > 0 ? JSON.parse(json) : null
            dialog.message = json.length > 0 ? "" : HermesApp.error
            if (dialog.info) {
                const i = dialog.info.providers.findIndex(p => p.supports_password)
                providerBox.currentIndex = Math.max(0, i)
            }
        }
        function onSignInFinished(ok, message) {
            if (ok) {
                dialog.clearForm()
                dialog.close()
            } else {
                dialog.message = message
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 10

        Label {
            text: "Saved servers"
            font.bold: true
            visible: dialog.servers.length > 0
        }
        Repeater {
            model: dialog.servers
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                Label {
                    Layout.fillWidth: true
                    text: modelData.name + "  —  " + modelData.url
                    elide: Text.ElideRight
                    font.bold: modelData.id === HermesApp.activeServer
                }
                Button { text: "Sign in again"; onClicked: dialog.edit(modelData) }
                Button { text: "Remove"; onClicked: HermesApp.removeServer(modelData.id) }
            }
        }

        MenuSeparator { Layout.fillWidth: true; visible: dialog.servers.length > 0 }

        Label { text: "Add a server"; font.bold: true }

        RowLayout {
            Layout.fillWidth: true
            TextField {
                id: urlField
                Layout.fillWidth: true
                placeholderText: "https://hermes.example.com"
                onAccepted: HermesApp.probeServer(text)
                onTextEdited: dialog.info = null
            }
            Button {
                text: "Check"
                enabled: urlField.text.trim().length > 0 && !HermesApp.busy
                onClicked: HermesApp.probeServer(urlField.text)
            }
        }

        // Shown once the server answered.
        ColumnLayout {
            Layout.fillWidth: true
            visible: dialog.info !== null
            spacing: 8

            Label {
                text: dialog.info ? "Hermes " + dialog.info.version + (dialog.info.auth_required ? "" : " (no sign-in required)") : ""
                opacity: 0.7
            }
            TextField {
                id: nameField
                Layout.fillWidth: true
                placeholderText: "Name (optional)"
            }
            ComboBox {
                id: providerBox
                Layout.fillWidth: true
                visible: dialog.info && dialog.info.providers.length > 1
                model: dialog.info ? dialog.info.providers : []
                textRole: "display_name"
            }
            TextField {
                id: userField
                Layout.fillWidth: true
                visible: dialog.usesPassword
                placeholderText: "Username"
            }
            TextField {
                id: passwordField
                Layout.fillWidth: true
                visible: dialog.usesPassword
                placeholderText: "Password"
                echoMode: TextInput.Password
                onAccepted: signInButton.clicked()
            }
            Label {
                Layout.fillWidth: true
                visible: dialog.info && !dialog.usesPassword && dialog.info.auth_required
                text: "Your browser will open to sign in."
                wrapMode: Text.Wrap
                opacity: 0.7
            }
            Button {
                id: signInButton
                text: HermesApp.busy ? "Signing in…" : "Sign in"
                enabled: !HermesApp.busy && (!dialog.usesPassword || (userField.text.length > 0 && passwordField.text.length > 0))
                onClicked: {
                    dialog.message = ""
                    HermesApp.signIn(urlField.text, nameField.text,
                                     dialog.provider ? dialog.provider.name : "",
                                     dialog.usesPassword, userField.text, passwordField.text)
                }
            }
        }

        BusyIndicator {
            running: HermesApp.busy
            visible: running
            Layout.alignment: Qt.AlignHCenter
        }

        Label {
            Layout.fillWidth: true
            visible: dialog.message.length > 0
            text: dialog.message
            color: "red"
            wrapMode: Text.Wrap
        }
    }
}
