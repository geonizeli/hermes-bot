//! A bot's live screen, painted from the RFB framebuffer, with input
//! forwarded while the user holds the control lease.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use super::queue::Queue;
use super::theme::colors;
use super::widgets;
use crate::backend::rfb::{self, Input, SharedFrame};
use crate::backend::{runtime, session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenState {
    Idle,
    Connecting,
    Live,
    Ended,
}

pub struct ScreenView {
    pub profile: String,
    pub state: ScreenState,
    pub message: String,
    pub in_control: bool,
    pub frame_width: u32,
    pub frame_height: u32,

    frame: SharedFrame,
    image: Option<Arc<RenderImage>>,
    input: Option<mpsc::UnboundedSender<Input>>,
    viewer_id: String,
    /// Bumped on attach/detach so a finished session's callbacks are ignored.
    generation: u64,
    wanted: bool,
    retries: u32,
    buttons: u8,
    last_position: (u16, u16),
    /// Keys held on the remote side, by the local key name.
    keys_down: HashMap<String, u32>,
    modifiers: Modifiers,
    repaint_queued: Arc<AtomicBool>,
    /// Where the view was last laid out, for mapping the pointer.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
    queue: Queue<ScreenView>,
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

impl Drop for ScreenView {
    /// Quitting while in control must hand the screen back: the server keeps
    /// a lease across a dropped link, and the bot can't use its computer
    /// until it does.
    fn drop(&mut self) {
        if !self.in_control || self.viewer_id.is_empty() {
            return;
        }
        let Some(session) = session::current() else { return };
        let params = json!({ "profile": self.profile, "viewer_id": self.viewer_id });
        let _ = runtime().block_on(async {
            tokio::time::timeout(std::time::Duration::from_millis(1500), session.gateway.request("display.lease.release", params)).await
        });
    }
}

impl ScreenView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        ScreenView {
            profile: String::new(),
            state: ScreenState::Idle,
            message: String::new(),
            in_control: false,
            frame_width: 0,
            frame_height: 0,
            frame: SharedFrame::default(),
            image: None,
            input: None,
            viewer_id: String::new(),
            generation: 0,
            wanted: false,
            retries: 0,
            buttons: 0,
            last_position: (0, 0),
            keys_down: HashMap::new(),
            modifiers: Modifiers::default(),
            repaint_queued: Arc::new(AtomicBool::new(false)),
            bounds: Rc::new(Cell::new(Bounds::default())),
            focus: cx.focus_handle(),
            queue: Queue::new(cx),
        }
    }

    /// The framebuffer's placement inside the view: scaled to fit, centered.
    fn target(&self, fw: u32, fh: u32) -> (f32, f32, f32) {
        let b = self.bounds.get();
        let (w, h) = (b.size.width.as_f32(), b.size.height.as_f32());
        if fw == 0 || fh == 0 || w <= 0.0 || h <= 0.0 {
            return (0.0, 0.0, 1.0);
        }
        let scale = (w / fw as f32).min(h / fh as f32);
        ((w - fw as f32 * scale) / 2.0, (h - fh as f32 * scale) / 2.0, scale)
    }

    /// Copies the framebuffer into a new image for the GPU.
    fn repaint(&mut self, cx: &mut Context<Self>) {
        let image = {
            let frame = self.frame.lock().unwrap();
            if frame.width == 0 || frame.pixels.is_empty() {
                return;
            }
            // 0xffRRGGBB little-endian is B, G, R, A in memory: the BGRA the GPU wants.
            let bytes: Vec<u8> = frame.pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
            let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, bytes) else { return };
            Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
        };
        if let Some(old) = self.image.replace(image) {
            cx.drop_image(old, None);
        }
        cx.notify();
    }

    fn set_lease(&mut self, lease: Value) {
        self.in_control = !self.viewer_id.is_empty()
            && lease.get("holder").and_then(Value::as_str) == Some("human")
            && lease.get("viewer_hash").and_then(Value::as_str) == Some(viewer_hash(&self.viewer_id).as_str());
    }

    /// The lease from a `display.status`/`display.lease`.
    pub fn apply_lease(&mut self, lease: &Value, cx: &mut Context<Self>) {
        if lease.is_object() {
            let was = self.in_control;
            self.set_lease(lease.clone());
            if was != self.in_control {
                cx.notify();
            }
        }
    }

    pub fn attach(&mut self, profile: &str, cx: &mut Context<Self>) {
        self.profile = profile.to_string();
        self.wanted = true;
        if self.input.is_some() {
            return;
        }
        let Some(session) = session::current() else {
            self.state = ScreenState::Ended;
            self.message = "Not connected.".into();
            cx.notify();
            return;
        };
        let profile = self.profile.clone();
        let viewer_id = self.viewer_id.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        self.generation += 1;
        self.input = Some(tx);
        let generation = self.generation;
        self.state = ScreenState::Connecting;
        self.message.clear();
        let frame = self.frame.clone();
        let repaint = self.repaint_queued.clone();
        let queue = self.queue.clone();

        runtime().spawn(async move {
            let mut params = json!({ "profile": profile });
            if !viewer_id.is_empty() {
                params["viewer_id"] = json!(viewer_id);
            }
            let observed = match session.gateway.request("display.observe", params).await {
                Ok(v) => v,
                Err(e) => {
                    queue.queue(move |view, cx| {
                        if view.generation == generation {
                            view.input = None;
                            view.state = ScreenState::Ended;
                            view.message = e;
                            cx.notify();
                        }
                    });
                    return;
                }
            };
            let text = |key: &str| observed.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            let path = Some(text("path")).filter(|p| !p.is_empty()).unwrap_or_else(|| "/api/display/ws".into());
            let mut url = match url::Url::parse(&format!("{}{}", ws_base(&session.base_url), path)) {
                Ok(url) => url,
                Err(e) => {
                    queue.queue(move |view, cx| {
                        view.input = None;
                        view.state = ScreenState::Ended;
                        view.message = format!("Bad screen address: {e}");
                        cx.notify();
                    });
                    return;
                }
            };
            url.query_pairs_mut().append_pair("display_ticket", &text("ticket"));
            let viewer = text("viewer_id");
            let lease = observed.get("lease").cloned().unwrap_or(Value::Null);
            queue.queue(move |view, cx| {
                if view.generation == generation {
                    if !viewer.is_empty() {
                        view.viewer_id = viewer;
                    }
                    view.set_lease(lease);
                    cx.notify();
                }
            });

            let events = queue.clone();
            let end = rfb::run(url.as_str(), frame, rx, move |event| match event {
                rfb::Event::Ready { width, height } => {
                    events.queue(move |view, cx| {
                        if view.generation == generation {
                            view.retries = 0;
                            view.frame_width = width;
                            view.frame_height = height;
                            view.state = ScreenState::Live;
                            view.repaint(cx);
                            cx.notify();
                        }
                    });
                }
                // Coalesce: one repaint in flight at a time.
                rfb::Event::Updated => {
                    if !repaint.swap(true, Ordering::AcqRel) {
                        let flag = repaint.clone();
                        events.queue(move |view, cx| {
                            flag.store(false, Ordering::Release);
                            view.repaint(cx);
                        });
                    }
                }
            })
            .await;

            queue.queue(move |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.input = None;
                match end.code {
                    // Someone else took control, or our ticket lapsed: watch again.
                    Some(4000) | Some(4401) if view.wanted && view.retries < 3 => {
                        view.retries += 1;
                        if end.code == Some(4000) {
                            view.message = "Someone else took control. Watching.".into();
                        }
                        let profile = view.profile.clone();
                        view.attach(&profile, cx);
                    }
                    Some(4001) => {
                        view.state = ScreenState::Ended;
                        view.message = "The screen stopped.".into();
                    }
                    _ => {
                        view.state = ScreenState::Ended;
                        view.message = if end.reason.is_empty() { "The screen connection ended.".to_string() } else { end.reason };
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    pub fn detach(&mut self, cx: &mut Context<Self>) {
        self.wanted = false;
        self.generation += 1;
        // Dropping the sender closes the socket cleanly, which also hands
        // the control lease back to the bot.
        self.input = None;
        self.keys_down.clear();
        self.buttons = 0;
        self.in_control = false;
        self.state = ScreenState::Idle;
        cx.notify();
    }

    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        let profile = self.profile.clone();
        self.detach(cx);
        self.attach(&profile, cx);
    }

    fn lease_call(&mut self, method: &'static str, cx: &mut Context<Self>) {
        let Some(session) = session::current() else { return };
        if self.viewer_id.is_empty() {
            self.message = "The screen isn't connected yet.".into();
            cx.notify();
            return;
        }
        let mut params = json!({ "profile": self.profile, "viewer_id": self.viewer_id });
        // Only acquire takes a reason; release rejects unknown fields.
        if method == "display.lease.acquire" {
            params["reason"] = json!("Taken from Hermes Bot");
        }
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = session.gateway.request(method, params).await;
            queue.queue(move |view, cx| {
                match result {
                    Ok(reply) => {
                        if let Some(lease) = reply.get("lease").cloned() {
                            view.set_lease(lease);
                        }
                        view.message.clear();
                    }
                    Err(e) => view.message = e,
                }
                cx.notify();
            });
        });
    }

    pub fn take_control(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.lease_call("display.lease.acquire", cx);
    }

    pub fn give_back(&mut self, cx: &mut Context<Self>) {
        // Let go of anything still held on the remote side first.
        let held: Vec<u32> = self.keys_down.drain().map(|(_, sym)| sym).collect();
        for keysym in held {
            self.send(Input::Key { down: false, keysym });
        }
        self.lease_call("display.lease.release", cx);
    }

    fn send(&self, input: Input) {
        if let Some(tx) = &self.input {
            let _ = tx.send(input);
        }
    }

    fn pointer(&mut self, position: Point<Pixels>) {
        if !self.in_control || self.frame_width == 0 || self.frame_height == 0 {
            return;
        }
        let (fw, fh) = (self.frame_width, self.frame_height);
        let origin = self.bounds.get().origin;
        let (ox, oy, scale) = self.target(fw, fh);
        let x = (position.x - origin.x).as_f32();
        let y = (position.y - origin.y).as_f32();
        let fx = ((x - ox) / scale).clamp(0.0, (fw - 1) as f32) as u16;
        let fy = ((y - oy) / scale).clamp(0.0, (fh - 1) as f32) as u16;
        self.last_position = (fx, fy);
        self.send(Input::Pointer { mask: self.buttons, x: fx, y: fy });
    }

    /// RFB button bits: left 0, middle 1, right 2.
    fn button_bit(button: MouseButton) -> u8 {
        match button {
            MouseButton::Left => 1,
            MouseButton::Middle => 2,
            MouseButton::Right => 4,
            _ => 0,
        }
    }

    fn wheel(&mut self, position: Point<Pixels>, delta: f32) {
        if !self.in_control || delta == 0.0 {
            return;
        }
        self.pointer(position);
        let (fx, fy) = self.last_position;
        let held = self.buttons;
        // Wheel up is button 4 (bit 3), down is button 5 (bit 4): a click each notch.
        let bit = if delta > 0.0 { 1 << 3 } else { 1 << 4 };
        for _ in 0..(delta.abs().round() as u32).max(1) {
            self.send(Input::Pointer { mask: held | bit, x: fx, y: fy });
            self.send(Input::Pointer { mask: held, x: fx, y: fy });
        }
    }

    fn key(&mut self, keystroke: &Keystroke, down: bool) -> bool {
        if !self.in_control {
            return false;
        }
        // A release must name the same keysym as its press, whatever the text says now.
        let keysym = if down {
            match rfb::keysym(&keystroke.key, keystroke.key_char.as_deref()) {
                Some(sym) => {
                    self.keys_down.insert(keystroke.key.clone(), sym);
                    sym
                }
                None => return false,
            }
        } else {
            match self.keys_down.remove(&keystroke.key) {
                Some(sym) => sym,
                None => match rfb::keysym(&keystroke.key, keystroke.key_char.as_deref()) {
                    Some(sym) => sym,
                    None => return false,
                },
            }
        };
        self.send(Input::Key { down, keysym });
        true
    }

    fn modifiers_changed(&mut self, now: Modifiers) {
        if !self.in_control {
            self.modifiers = now;
            return;
        }
        let before = self.modifiers;
        for (was, is, sym) in [
            (before.shift, now.shift, 0xffe1),
            (before.control, now.control, 0xffe3),
            (before.alt, now.alt, 0xffe9),
            (before.platform, now.platform, 0xffeb),
        ] {
            if was != is {
                self.send(Input::Key { down: is, keysym: sym });
            }
        }
        self.modifiers = now;
    }
}

impl Render for ScreenView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let bounds = self.bounds.clone();
        let in_control = self.in_control;
        div()
            .id("screen-view")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(black())
            .overflow_hidden()
            .when_some(self.image.clone(), |d, image| {
                d.child(img(ImageSource::from(image)).size_full().object_fit(ObjectFit::Contain))
            })
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .when(self.state == ScreenState::Connecting, |d| {
                d.child(div().absolute().inset_0().flex().items_center().justify_center().child(widgets::spinner(32.)))
            })
            .when(in_control, |d| {
                d.cursor(CursorStyle::Crosshair)
                    .child(div().absolute().inset_0().border_2().border_color(c.accent))
            })
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| this.mouse(e.button, true, e.position, window, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, e: &MouseDownEvent, window, cx| this.mouse(e.button, true, e.position, window, cx)))
            .on_mouse_down(MouseButton::Middle, cx.listener(|this, e: &MouseDownEvent, window, cx| this.mouse(e.button, true, e.position, window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, window, cx| this.mouse(e.button, false, e.position, window, cx)))
            .on_mouse_up(MouseButton::Right, cx.listener(|this, e: &MouseUpEvent, window, cx| this.mouse(e.button, false, e.position, window, cx)))
            .on_mouse_up(MouseButton::Middle, cx.listener(|this, e: &MouseUpEvent, window, cx| this.mouse(e.button, false, e.position, window, cx)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, _| this.pointer(e.position)))
            .on_scroll(cx.listener(|this, e: &ScrollEvent, _, _| {
                let notches = match e.delta {
                    ScrollDelta::Lines(l) => l.y,
                    ScrollDelta::Pixels(p) => p.y.as_f32() / 40.0,
                };
                this.wheel(e.position, notches);
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if this.key(&e.keystroke, true) {
                    cx.stop_propagation();
                }
            }))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, cx| {
                if this.key(&e.keystroke, false) {
                    cx.stop_propagation();
                }
            }))
            .on_modifiers_changed(cx.listener(|this, e: &ModifiersChangedEvent, _, _| this.modifiers_changed(e.modifiers)))
    }
}

impl ScreenView {
    fn mouse(&mut self, button: MouseButton, down: bool, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.in_control {
            return;
        }
        if down {
            self.focus.focus(window, cx);
            self.buttons |= Self::button_bit(button);
        } else {
            self.buttons &= !Self::button_bit(button);
        }
        self.pointer(position);
        cx.stop_propagation();
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
