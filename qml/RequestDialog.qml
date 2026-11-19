import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import HermesBot

// Answers the questions a bot asks mid-turn: command approvals, clarifying
// questions, and masked values (sudo, secrets, vault prompts).
Dialog {
    id: dialog
    modal: true
    closePolicy: Popup.NoAutoClose

    enter: Transition {
        NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 160; easing.type: Easing.OutCubic }
        NumberAnimation { property: "scale"; from: 0.96; to: 1; duration: 160; easing.type: Easing.OutCubic }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; from: 1; to: 0; duration: 120 }
        NumberAnimation { property: "scale"; from: 1; to: 0.97; duration: 120 }
    }
    width: Math.min(560, parent ? parent.width - 40 : 560)

    property string requestId: ""
    property string method: ""
    property var params: ({})
    property var answers: ({})

    readonly property bool isApproval: method === "approval"
    readonly property bool isClarify: method === "clarify"
    readonly property bool isValue: ["sudo", "secret", "vault.code", "vault.unlock_prompt", "display.install.sudo"].indexOf(method) >= 0

    function open_(id, m, p) {
        // Requests that only make sense inside the official desktop app are
        // declined at once rather than leaving the bot waiting.
        if (!["approval", "clarify", "sudo", "secret", "vault.code", "vault.unlock_prompt", "display.install.sudo"].includes(m)) {
            HermesApp.answerRequest(id, JSON.stringify({ value: "" }))
            return
        }
        requestId = id
        method = m
        params = p
        answers = ({})
        valueField.text = ""
        title = isApproval ? "Approve command?"
              : isClarify ? "The bot has a question"
              : (p.prompt || (m === "sudo" ? "Sudo password" : "Value needed"))
        open()
    }

    function answer(result) {
        HermesApp.answerRequest(requestId, JSON.stringify(result))
        close()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 10

        // Approval
        Label {
            visible: dialog.isApproval && (dialog.params.description || "").length > 0
            Layout.fillWidth: true
            text: dialog.params.description || ""
            wrapMode: Text.Wrap
        }
        TextArea {
            visible: dialog.isApproval || (dialog.method === "sudo" && (dialog.params.command || "").length > 0)
            Layout.fillWidth: true
            readOnly: true
            text: dialog.params.command || ""
            wrapMode: TextEdit.Wrap
            font.family: "monospace"
        }
        Flow {
            visible: dialog.isApproval
            Layout.fillWidth: true
            spacing: 8
            Repeater {
                model: dialog.isApproval ? (dialog.params.choices || ["once", "session", "always", "deny"]) : []
                delegate: Button {
                    required property string modelData
                    text: ({ once: "Allow once", session: "Allow for this chat", always: "Always allow", deny: "Deny" })[modelData] || modelData
                    onClicked: dialog.answer({ choice: modelData })
                }
            }
        }

        // Clarify: one field per question, or a choice list.
        Repeater {
            model: dialog.isClarify ? (dialog.params.questions || []) : []
            delegate: ColumnLayout {
                required property var modelData
                Layout.fillWidth: true
                Label {
                    Layout.fillWidth: true
                    text: modelData.question
                    wrapMode: Text.Wrap
                    font.bold: true
                }
                ComboBox {
                    visible: (modelData.choices || []).length > 0
                    Layout.fillWidth: true
                    model: modelData.choices || []
                    onActivated: dialog.answers[modelData.qid] = currentText
                    Component.onCompleted: if (visible) dialog.answers[modelData.qid] = currentText
                }
                TextField {
                    visible: (modelData.choices || []).length === 0
                    Layout.fillWidth: true
                    onTextChanged: dialog.answers[modelData.qid] = text
                }
            }
        }
        RowLayout {
            visible: dialog.isClarify
            Item { Layout.fillWidth: true }
            Button { text: "Skip"; onClicked: dialog.answer({ outcome: "cancelled" }) }
            Button { text: "Answer"; onClicked: dialog.answer({ answers: dialog.answers, outcome: "submitted" }) }
        }

        // Masked value.
        TextField {
            id: valueField
            visible: dialog.isValue
            Layout.fillWidth: true
            echoMode: TextInput.Password
            onAccepted: dialog.answer({ value: text })
        }
        RowLayout {
            visible: dialog.isValue
            Item { Layout.fillWidth: true }
            Button { text: "Decline"; onClicked: dialog.answer({ value: "" }) }
            Button { text: "Send"; onClicked: dialog.answer({ value: valueField.text }) }
        }
    }
}
