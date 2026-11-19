//! hermes-bot: chat with Hermes bots on remote Hermes servers.

mod bridge;
mod backend;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};

fn main() {
    let mut app = QGuiApplication::new();
    if let Some(mut app) = app.as_mut() {
        app.as_mut().set_application_name(&QString::from("hermes-bot"));
        app.as_mut().set_application_version(&QString::from(env!("CARGO_PKG_VERSION")));
        // QSettings (QML Settings) keeps UI state in ~/.config/hermes-bot/.
        app.as_mut().set_organization_name(&QString::from("hermes-bot"));
    }
    // Matches the window to hermes-bot.desktop, so Hyprland (Wayland app_id)
    // and the launcher pick up the installed icon.
    QGuiApplication::set_desktop_file_name(&QString::from("hermes-bot"));

    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/HermesBot/qml/Main.qml"));
    }

    let code = app.as_mut().map(|app| app.exec()).unwrap_or(1);
    // Tear the QML scene down before exiting so objects can clean up (the
    // screen viewer hands back control of a bot's screen).
    drop(engine);
    drop(app);
    std::process::exit(code);
}
