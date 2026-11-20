pub mod attachments;
pub mod auth;
pub mod botchat;
pub mod cron;
pub mod gateway;
pub mod history;
pub mod markdown;
pub mod media;
pub mod rfb;
pub mod session;
pub mod store;
pub mod theme;

use std::sync::OnceLock;

/// One tokio runtime for all network work. Results are posted back to the
/// Qt thread through each QObject's `qt_thread()`.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("hermes-bot-net")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}
