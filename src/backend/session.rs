//! The live connection, shared with QObjects other than `HermesApp` (the
//! screen viewer needs the same `/api/ws` link for its tickets and lease).

use std::sync::{Arc, Mutex};

use super::gateway::Gateway;

#[derive(Clone)]
pub struct Session {
    pub gateway: Arc<Gateway>,
    pub base_url: String,
}

static CURRENT: Mutex<Option<Session>> = Mutex::new(None);

pub fn set(session: Option<Session>) {
    *CURRENT.lock().unwrap() = session;
}

pub fn current() -> Option<Session> {
    CURRENT.lock().unwrap().clone()
}
