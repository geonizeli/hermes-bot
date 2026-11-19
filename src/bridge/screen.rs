//! `ScreenView`: a bot's live screen, painted from the RFB framebuffer, with
//! input forwarded while the user holds the control lease.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qpainter.h");
        type QPainter = cxx_qt_lib::QPainter;
        include!(<QtQuick/QQuickPaintedItem>);
        type QQuickPaintedItem;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QQuickPaintedItem]
        #[qproperty(QString, profile)]
        /// idle, connecting, live or ended.
        #[qproperty(QString, state)]
        #[qproperty(QString, message)]
        #[qproperty(bool, in_control, cxx_name = "inControl")]
        #[qproperty(i32, frame_width, cxx_name = "frameWidth")]
        #[qproperty(i32, frame_height, cxx_name = "frameHeight")]
        type ScreenView = super::ScreenViewRust;

        #[cxx_name = "paint"]
        #[cxx_override]
        unsafe fn paint(self: Pin<&mut ScreenView>, painter: *mut QPainter);

        #[inherit]
        fn update(self: Pin<&mut ScreenView>);
        #[inherit]
        fn width(self: &ScreenView) -> f64;
        #[inherit]
        fn height(self: &ScreenView) -> f64;

        #[qinvokable]
        fn attach(self: Pin<&mut ScreenView>);
        #[qinvokable]
        fn detach(self: Pin<&mut ScreenView>);
        #[qinvokable]
        fn take_control(self: Pin<&mut ScreenView>);
        #[qinvokable]
        fn give_back(self: Pin<&mut ScreenView>);
        /// The lease from a `display.status`/`display.lease` (JSON).
        #[qinvokable]
        fn apply_lease(self: Pin<&mut ScreenView>, lease: QString);
        /// Pointer at item coordinates with Qt mouse buttons.
        #[qinvokable]
        fn pointer(self: Pin<&mut ScreenView>, x: f64, y: f64, buttons: i32);
        #[qinvokable]
        fn wheel(self: Pin<&mut ScreenView>, x: f64, y: f64, delta: i32);
        /// Returns whether the key was sent.
        #[qinvokable]
        fn key(self: Pin<&mut ScreenView>, key: i32, text: QString, down: bool) -> bool;
        #[qinvokable]
        fn paste(self: Pin<&mut ScreenView>, text: QString);
    }

    impl cxx_qt::Threading for ScreenView {}
    impl cxx_qt::Constructor<()> for ScreenView {}
}

use core::pin::Pin;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QImage, QImageFormat, QPainterRenderHint, QRect, QString};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::backend::rfb::{self, Input, SharedFrame};
use crate::backend::{runtime, session};

#[derive(Default)]
pub struct ScreenViewRust {
    profile: QString,
    state: QString,
    message: QString,
    in_control: bool,
    frame_width: i32,
    frame_height: i32,

    frame: SharedFrame,
    input: Option<mpsc::UnboundedSender<Input>>,
    viewer_id: String,
    lease: Value,
    /// Bumped on attach/detach so a finished session's callbacks are ignored.
    generation: u64,
    wanted: bool,
    retries: u32,
    buttons: u8,
    last_position: (u16, u16),
    keys_down: HashMap<i32, u32>,
    repaint_queued: Arc<AtomicBool>,
}

impl cxx_qt::Constructor<()> for qobject::ScreenView {
    type NewArguments = ();
    type BaseArguments = ();
    type InitializeArguments = ();

    fn route_arguments(_: ()) -> ((), (), ()) {
        ((), (), ())
    }

    fn new(_: ()) -> ScreenViewRust {
        let mut view = ScreenViewRust::default();
        view.state = QString::from("idle");
        view
    }
}

fn q(s: impl AsRef<str>) -> QString {
    QString::from(s.as_ref())
}

/// `wss://host/prefix` from a dashboard base URL.
fn ws_base(base: &str) -> String {
    if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    }
}

/// The lease's `viewer_hash` is the first 12 hex digits of sha256(viewer_id).
pub fn viewer_hash(viewer_id: &str) -> String {
    Sha256::digest(viewer_id.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

extern "C" fn keep_buffer(_: *mut cxx_qt_lib::c_void) {}

impl Drop for ScreenViewRust {
    /// Quitting while in control must hand the screen back: the server keeps
    /// a lease across a dropped link, and the bot can't use its computer
    /// until it does.
    fn drop(&mut self) {
        if !self.in_control || self.viewer_id.is_empty() {
            return;
        }
        let Some(session) = session::current() else { return };
        let params = json!({ "profile": self.profile.to_string(), "viewer_id": self.viewer_id });
        let _ = runtime().block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_millis(1500),
                session.gateway.request("display.lease.release", params),
            )
            .await
        });
    }
}

impl qobject::ScreenView {
    /// The framebuffer's placement inside the item: scaled to fit, centered.
    fn target(&self, fw: u32, fh: u32) -> (f64, f64, f64) {
        let (w, h) = (self.width(), self.height());
        if fw == 0 || fh == 0 || w <= 0.0 || h <= 0.0 {
            return (0.0, 0.0, 1.0);
        }
        let scale = (w / fw as f64).min(h / fh as f64);
        ((w - fw as f64 * scale) / 2.0, (h - fh as f64 * scale) / 2.0, scale)
    }

    unsafe fn paint(self: Pin<&mut Self>, painter: *mut cxx_qt_lib::QPainter) {
        let Some(painter) = (unsafe { painter.as_mut() }) else { return };
        let mut painter = unsafe { Pin::new_unchecked(painter) };
        let frame = self.rust().frame.clone();
        let frame = frame.lock().unwrap();
        if frame.width == 0 || frame.pixels.is_empty() {
            return;
        }
        let (x, y, scale) = self.target(frame.width, frame.height);
        // Wraps the framebuffer without copying; it lives until drawn, under the lock.
        let image = unsafe {
            QImage::from_raw_parts(
                frame.pixels.as_ptr().cast(),
                frame.width as i32,
                frame.height as i32,
                QImageFormat::Format_RGB32,
                keep_buffer,
                std::ptr::null_mut(),
            )
        };
        painter.as_mut().set_render_hint(QPainterRenderHint::SmoothPixmapTransform, true);
        let rect = QRect::new(
            x.round() as i32,
            y.round() as i32,
            (frame.width as f64 * scale).round() as i32,
            (frame.height as f64 * scale).round() as i32,
        );
        painter.as_mut().draw_image(&rect, &image);
        drop(image);
    }

    fn set_lease_value(mut self: Pin<&mut Self>, lease: Value) {
        let mine = !self.rust().viewer_id.is_empty()
            && lease.get("holder").and_then(Value::as_str) == Some("human")
            && lease.get("viewer_hash").and_then(Value::as_str) == Some(viewer_hash(&self.rust().viewer_id).as_str());
        self.as_mut().rust_mut().lease = lease;
        self.as_mut().set_in_control(mine);
    }

    pub fn apply_lease(self: Pin<&mut Self>, lease: QString) {
        if let Ok(value) = serde_json::from_str::<Value>(&lease.to_string()) {
            if value.is_object() {
                self.set_lease_value(value);
            }
        }
    }

    pub fn attach(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().wanted = true;
        if self.rust().input.is_some() {
            return;
        }
        let Some(session) = session::current() else {
            self.as_mut().set_state(q("ended"));
            self.as_mut().set_message(q("Not connected."));
            return;
        };
        let profile = self.profile().to_string();
        let viewer_id = self.rust().viewer_id.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        let generation = {
            let mut rust = self.as_mut().rust_mut();
            rust.generation += 1;
            rust.input = Some(tx);
            rust.generation
        };
        self.as_mut().set_state(q("connecting"));
        self.as_mut().set_message(q(""));
        let frame = self.rust().frame.clone();
        let repaint = self.rust().repaint_queued.clone();
        let qt = self.qt_thread();

        runtime().spawn(async move {
            let mut params = json!({ "profile": profile });
            if !viewer_id.is_empty() {
                params["viewer_id"] = json!(viewer_id);
            }
            let observed = match session.gateway.request("display.observe", params).await {
                Ok(v) => v,
                Err(e) => {
                    let _ = qt.queue(move |mut view| {
                        if view.rust().generation == generation {
                            view.as_mut().rust_mut().input = None;
                            view.as_mut().set_state(q("ended"));
                            view.as_mut().set_message(q(e));
                        }
                    });
                    return;
                }
            };
            let text = |key: &str| observed.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            let path = Some(text("path")).filter(|p| !p.is_empty()).unwrap_or_else(|| "/api/display/ws".into());
            let mut url = url::Url::parse(&format!("{}{}", ws_base(&session.base_url), path)).expect("valid screen url");
            url.query_pairs_mut().append_pair("display_ticket", &text("ticket"));
            let viewer = text("viewer_id");
            let lease = observed.get("lease").cloned().unwrap_or(Value::Null);
            let qt_setup = qt.clone();
            let _ = qt_setup.queue(move |mut view| {
                if view.rust().generation == generation {
                    if !viewer.is_empty() {
                        view.as_mut().rust_mut().viewer_id = viewer;
                    }
                    view.as_mut().set_lease_value(lease);
                }
            });

            let qt_events = qt.clone();
            let end = rfb::run(url.as_str(), frame, rx, move |event| match event {
                rfb::Event::Ready { width, height, .. } => {
                    let _ = qt_events.queue(move |mut view| {
                        if view.rust().generation == generation {
                            view.as_mut().rust_mut().retries = 0;
                            view.as_mut().set_frame_width(width as i32);
                            view.as_mut().set_frame_height(height as i32);
                            view.as_mut().set_state(q("live"));
                            view.update();
                        }
                    });
                }
                // Coalesce: one repaint in flight at a time.
                rfb::Event::Updated => {
                    if !repaint.swap(true, Ordering::AcqRel) {
                        let flag = repaint.clone();
                        let _ = qt_events.queue(move |view| {
                            flag.store(false, Ordering::Release);
                            view.update();
                        });
                    }
                }
            })
            .await;

            let _ = qt.queue(move |mut view| {
                if view.rust().generation != generation {
                    return;
                }
                view.as_mut().rust_mut().input = None;
                let wanted = view.rust().wanted;
                let retries = view.rust().retries;
                match end.code {
                    // Someone else took control, or our ticket lapsed: watch again.
                    Some(4000) | Some(4401) if wanted && retries < 3 => {
                        view.as_mut().rust_mut().retries += 1;
                        if end.code == Some(4000) {
                            view.as_mut().set_message(q("Someone else took control. Watching."));
                        }
                        view.attach();
                    }
                    Some(4001) => {
                        view.as_mut().set_state(q("ended"));
                        view.as_mut().set_message(q("The screen stopped."));
                    }
                    _ => {
                        view.as_mut().set_state(q("ended"));
                        view.as_mut().set_message(q(if end.reason.is_empty() {
                            "The screen connection ended.".to_string()
                        } else {
                            end.reason
                        }));
                    }
                }
            });
        });
    }

    pub fn detach(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.wanted = false;
            rust.generation += 1;
            // Dropping the sender closes the socket cleanly, which also hands
            // the control lease back to the bot.
            rust.input = None;
            rust.keys_down.clear();
            rust.buttons = 0;
        }
        self.as_mut().set_in_control(false);
        self.as_mut().set_state(q("idle"));
    }

    fn lease_call(mut self: Pin<&mut Self>, method: &'static str) {
        let Some(session) = session::current() else { return };
        let viewer_id = self.rust().viewer_id.clone();
        if viewer_id.is_empty() {
            self.as_mut().set_message(q("The screen isn't connected yet."));
            return;
        }
        let profile = self.profile().to_string();
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = session
                .gateway
                .request(method, json!({ "profile": profile, "viewer_id": viewer_id, "reason": "Taken from Hermes Bot" }))
                .await;
            let _ = qt.queue(move |mut view| match result {
                Ok(reply) => {
                    if let Some(lease) = reply.get("lease").cloned() {
                        view.as_mut().set_lease_value(lease);
                    }
                    view.as_mut().set_message(q(""));
                }
                Err(e) => view.as_mut().set_message(q(e)),
            });
        });
    }

    pub fn take_control(self: Pin<&mut Self>) {
        self.lease_call("display.lease.acquire");
    }

    pub fn give_back(mut self: Pin<&mut Self>) {
        // Let go of anything still held on the remote side first.
        let held: Vec<u32> = self.as_mut().rust_mut().keys_down.drain().map(|(_, sym)| sym).collect();
        for keysym in held {
            self.as_mut().send(Input::Key { down: false, keysym });
        }
        self.lease_call("display.lease.release");
    }

    fn send(self: Pin<&mut Self>, input: Input) {
        if let Some(tx) = &self.rust().input {
            let _ = tx.send(input);
        }
    }

    pub fn pointer(mut self: Pin<&mut Self>, x: f64, y: f64, buttons: i32) {
        if !self.rust().in_control {
            return;
        }
        let (fw, fh) = (self.rust().frame_width.max(0) as u32, self.rust().frame_height.max(0) as u32);
        if fw == 0 || fh == 0 {
            return;
        }
        let (ox, oy, scale) = self.target(fw, fh);
        let fx = ((x - ox) / scale).clamp(0.0, (fw - 1) as f64) as u16;
        let fy = ((y - oy) / scale).clamp(0.0, (fh - 1) as f64) as u16;
        // Qt: left 1, right 2, middle 4. RFB: left bit 0, middle bit 1, right bit 2.
        let mask = (buttons & 1 != 0) as u8 | ((buttons & 4 != 0) as u8) << 1 | ((buttons & 2 != 0) as u8) << 2;
        {
            let mut rust = self.as_mut().rust_mut();
            rust.buttons = mask;
            rust.last_position = (fx, fy);
        }
        self.send(Input::Pointer { mask, x: fx, y: fy });
    }

    pub fn wheel(mut self: Pin<&mut Self>, x: f64, y: f64, delta: i32) {
        if !self.rust().in_control || delta == 0 {
            return;
        }
        let held = self.rust().buttons;
        self.as_mut().pointer(x, y, 0);
        let (fx, fy) = self.rust().last_position;
        // Wheel up is button 4 (bit 3), down is button 5 (bit 4): a click each notch.
        let bit = if delta > 0 { 1 << 3 } else { 1 << 4 };
        for _ in 0..(delta.abs() / 120).max(1) {
            self.as_mut().send(Input::Pointer { mask: held | bit, x: fx, y: fy });
            self.as_mut().send(Input::Pointer { mask: held, x: fx, y: fy });
        }
    }

    pub fn key(mut self: Pin<&mut Self>, key: i32, text: QString, down: bool) -> bool {
        if !self.rust().in_control {
            return false;
        }
        // A release must name the same keysym as its press, whatever the text says now.
        let keysym = if down {
            match rfb::keysym(key, &text.to_string()) {
                Some(sym) => {
                    self.as_mut().rust_mut().keys_down.insert(key, sym);
                    sym
                }
                None => return false,
            }
        } else {
            match self.as_mut().rust_mut().keys_down.remove(&key) {
                Some(sym) => sym,
                None => match rfb::keysym(key, &text.to_string()) {
                    Some(sym) => sym,
                    None => return false,
                },
            }
        };
        self.send(Input::Key { down, keysym });
        true
    }

    pub fn paste(self: Pin<&mut Self>, text: QString) {
        if self.rust().in_control {
            self.send(Input::Paste(text.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hashes_viewer_ids_like_the_server() {
        // sha256("abc") = ba7816bf8f01cfea…
        assert_eq!(super::viewer_hash("abc"), "ba7816bf8f01");
    }
}
