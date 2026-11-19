//! hermes-bot: chat with Hermes bots on remote Hermes servers.

mod bridge;
mod backend;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};

fn main() {
    let mut app = QGuiApplication::new();
    if let Some(mut app) = app.as_mut() {
        app.as_mut().set_application_name(&QString::from("hermes-bot"));
        app.as_mut().set_application_version(&QString::from(env!("CARGO_PKG_VERSION")));
    }
    // Matches the window to hermes-bot.desktop, so Hyprland (Wayland app_id)
    // and the launcher pick up the installed icon.
    QGuiApplication::set_desktop_file_name(&QString::from("hermes-bot"));
    QQuickStyle::set_style(&QString::from("Material"));

    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/HermesBot/qml/Main.qml"));
    }

    if let Some(app) = app.as_mut() {
        std::process::exit(app.exec());
    }
}
