use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("HermesBot").qml_files([
        "qml/Main.qml",
        "qml/SetupDialog.qml",
        "qml/RequestDialog.qml",
    ]))
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .files(["src/bridge/theme.rs", "src/bridge/app.rs"])
    .build();
}
