use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("HermesBot").qml_files([
        "qml/Main.qml",
        "qml/BotAvatar.qml",
        "qml/ChatView.qml",
        "qml/Composer.qml",
        "qml/JobsPanel.qml",
        "qml/MarkdownView.qml",
        "qml/ModelPicker.qml",
        "qml/RemoteImage.qml",
        "qml/ScreenPanel.qml",
        "qml/SidePanel.qml",
        "qml/ToolCard.qml",
        "qml/Sidebar.qml",
        "qml/SetupDialog.qml",
        "qml/RequestDialog.qml",
    ]))
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .files(["src/bridge/theme.rs", "src/bridge/app.rs", "src/bridge/screen.rs"])
    .build();
}
