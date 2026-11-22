//! JSON-RPC 2.0 over the dashboard's `/api/ws` WebSocket (Hermes' tui_gateway
//! protocol): requests with replies, streamed events, and requests the server
//! sends to us (approvals, clarifying questions).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use super::auth;

#[derive(Debug, Clone)]
pub enum Incoming {
    Event { kind: String, session_id: String, payload: Value },
    ServerRequest { id: Value, method: String, params: Value },
    Closed(String),
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>;

pub struct Gateway {
    outgoing: mpsc::UnboundedSender<Message>,
    pending: Pending,
    next_id: AtomicU64,
}

/// Exchanges the bearer token for a single-use WebSocket ticket.
async fn ws_ticket(base: &str, access_token: &str) -> Result<String, String> {
    let response = auth::http()
        .post(format!("{base}/api/auth/ws-ticket"))
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Can't reach {base}: {e}"))?;
    if response.status().as_u16() == 401 {
        return Err(UNAUTHORIZED.into());
    }
    if !response.status().is_success() {
        return Err(format!("The server refused the chat connection (HTTP {}).", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    body.get("ticket")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "The server returned no chat ticket.".into())
}

pub const UNAUTHORIZED: &str = "unauthorized";

pub fn ws_url(base: &str, ticket: &str) -> String {
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    let mut url = url::Url::parse(&format!("{ws_base}/api/ws")).expect("valid ws url");
    url.query_pairs_mut().append_pair("ticket", ticket);
    url.into()
}

impl Gateway {
    /// Connects with `access_token`. A token the server rejects returns
    /// `Err(UNAUTHORIZED)` so the caller can refresh and retry.
    pub async fn connect(
        base: &str,
        access_token: &str,
        events: mpsc::UnboundedSender<Incoming>,
    ) -> Result<Arc<Gateway>, String> {
        let ticket = ws_ticket(base, access_token).await?;
        Self::connect_url(&ws_url(base, &ticket), events).await
    }

    pub async fn connect_url(url: &str, events: mpsc::UnboundedSender<Incoming>) -> Result<Arc<Gateway>, String> {
        let (socket, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(r) if r.status().as_u16() == 401 => UNAUTHORIZED.to_string(),
            e => format!("Chat connection failed: {e}"),
        })?;
        let (mut sink, mut stream) = socket.split();
        let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<Message>();
        let pending: Pending = Arc::default();

        tokio::spawn(async move {
            while let Some(message) = outgoing_rx.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let reader_pending = pending.clone();
        let reader_outgoing = outgoing.clone();
        tokio::spawn(async move {
            let reason = loop {
                match stream.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(frame) = serde_json::from_str::<Value>(&text) {
                            route(frame, &reader_pending, &events).await;
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = reader_outgoing.send(Message::Pong(data));
                    }
                    Some(Ok(Message::Close(frame))) => {
                        break match frame {
                            Some(f) if u16::from(f.code) == 4401 => UNAUTHORIZED.to_string(),
                            Some(f) => format!("The server closed the chat connection ({}: {}).", u16::from(f.code), f.reason),
                            None => "The server closed the chat connection.".to_string(),
                        };
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break format!("Chat connection lost: {e}"),
                    None => break "Chat connection lost.".to_string(),
                }
            };
            for (_, waiter) in reader_pending.lock().await.drain() {
                let _ = waiter.send(Err(reason.clone()));
            }
            let _ = events.send(Incoming::Closed(reason));
        });

        let gateway = Arc::new(Gateway { outgoing, pending, next_id: AtomicU64::new(1) });

        // The server expects a heartbeat; a missed reply means the link is dead.
        let weak = Arc::downgrade(&gateway);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(20)).await;
                let Some(gateway) = weak.upgrade() else { break };
                if gateway.request("gateway.ping", json!({})).await.is_err() {
                    let _ = gateway.outgoing.send(Message::Close(None));
                    break;
                }
            }
        });

        Ok(gateway)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.request_within(method, params, Duration::from_secs(60)).await
    }

    /// `request` with its own deadline, for calls the server takes long on.
    pub async fn request_within(&self, method: &str, params: Value, deadline: Duration) -> Result<Value, String> {
        let id = format!("hb{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if self.outgoing.send(Message::text(frame.to_string())).is_err() {
            self.pending.lock().await.remove(&id);
            return Err("Not connected.".into());
        }
        match tokio::time::timeout(deadline, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Not connected.".into()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("The server didn't answer {method} in time."))
            }
        }
    }

    /// Answers a request the server sent us.
    pub fn respond(&self, id: Value, result: Value) {
        let frame = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        let _ = self.outgoing.send(Message::text(frame.to_string()));
    }

    pub fn close(&self) {
        let _ = self.outgoing.send(Message::Close(None));
    }
}

async fn route(frame: Value, pending: &Pending, events: &mpsc::UnboundedSender<Incoming>) {
    let method = frame.get("method").and_then(Value::as_str);
    match method {
        None => {
            // A reply to one of our requests.
            let Some(id) = frame.get("id").map(|id| id.as_str().map(str::to_string).unwrap_or_else(|| id.to_string()))
            else {
                return;
            };
            if let Some(waiter) = pending.lock().await.remove(&id) {
                let result = match frame.get("error") {
                    Some(error) => Err(error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("The server reported an error.")
                        .to_string()),
                    None => Ok(frame.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = waiter.send(result);
            }
        }
        Some("event") => {
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            let text = |key: &str| params.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            let _ = events.send(Incoming::Event {
                kind: text("type"),
                session_id: text("session_id"),
                payload: params.get("payload").cloned().unwrap_or(Value::Null),
            });
        }
        Some(method) => {
            if let Some(id) = frame.get("id") {
                let _ = events.send(Incoming::ServerRequest {
                    id: id.clone(),
                    method: method.to_string(),
                    params: frame.get("params").cloned().unwrap_or(Value::Null),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn builds_websocket_urls() {
        assert_eq!(ws_url("https://h.example/pre", "t k"), "wss://h.example/pre/api/ws?ticket=t+k");
        assert_eq!(ws_url("http://10.0.0.2:9119", "t"), "ws://10.0.0.2:9119/api/ws?ticket=t");
    }

    /// A fake gateway: answers requests, pushes an event, and asks the client
    /// a question, the way tui_gateway does.
    #[tokio::test]
    async fn routes_replies_events_and_server_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            ws.send(Message::text(
                json!({"jsonrpc":"2.0","method":"event","params":{"type":"gateway.ready","payload":{}}}).to_string(),
            ))
            .await
            .unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let frame: Value = serde_json::from_str(&text).unwrap();
                if frame.get("method").is_none() {
                    // Our answer to the approval request.
                    assert_eq!(frame["result"]["choice"], "once");
                    break;
                }
                let id = frame["id"].clone();
                let reply = match frame["method"].as_str().unwrap() {
                    "session.create" => json!({"jsonrpc":"2.0","id":id,"result":{"session_id":"live1"}}),
                    _ => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"unknown method"}}),
                };
                ws.send(Message::text(reply.to_string())).await.unwrap();
                ws.send(Message::text(
                    json!({"jsonrpc":"2.0","method":"event","params":{"type":"message.delta","session_id":"live1","seq":1,"payload":{"text":"Hi"}}}).to_string(),
                ))
                .await
                .unwrap();
                ws.send(Message::text(
                    json!({"jsonrpc":"2.0","id":"srv1","method":"approval.request","params":{"request_id":"r"}}).to_string(),
                ))
                .await
                .unwrap();
            }
        });

        let (tx, mut rx) = mpsc::unbounded_channel();
        let gateway = Gateway::connect_url(&format!("ws://127.0.0.1:{port}/api/ws"), tx).await.unwrap();
        let ready = rx.recv().await.unwrap();
        assert!(matches!(ready, Incoming::Event { ref kind, .. } if kind == "gateway.ready"));

        let created = gateway.request("session.create", json!({})).await.unwrap();
        assert_eq!(created["session_id"], "live1");
        match rx.recv().await.unwrap() {
            Incoming::Event { kind, session_id, payload } => {
                assert_eq!((kind.as_str(), session_id.as_str()), ("message.delta", "live1"));
                assert_eq!(payload["text"], "Hi");
            }
            other => panic!("unexpected {other:?}"),
        }
        match rx.recv().await.unwrap() {
            Incoming::ServerRequest { id, method, .. } => {
                assert_eq!(method, "approval.request");
                gateway.respond(id, json!({"choice":"once"}));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(rx.recv().await.unwrap(), Incoming::Closed(_)));
    }
}

/// Drives a real server's display methods with the saved sign-in:
/// `HERMES_BOT_DISPLAY=start|stop|status HERMES_BOT_PROFILE=work cargo test display_live -- --ignored`.
#[cfg(test)]
mod live {
    use super::*;
    use crate::backend::store;

    #[tokio::test]
    #[ignore]
    async fn display_live() {
        let (Ok(action), Ok(profile)) = (std::env::var("HERMES_BOT_DISPLAY"), std::env::var("HERMES_BOT_PROFILE")) else {
            return;
        };
        let config = store::load(&store::config_path()).unwrap();
        let server = config.server(config.active_server.as_deref().unwrap()).unwrap().clone();
        let mut tokens = store::load_tokens(&server.id).expect("signed in");
        if tokens.expires_soon() {
            tokens = crate::backend::auth::refresh(&server.url, &tokens).await.ok().expect("refresh");
            store::save_tokens(&server.id, &tokens).unwrap();
        }
        let (tx, _rx) = mpsc::unbounded_channel();
        let gateway = Gateway::connect(&server.url, &tokens.access_token, tx).await.unwrap();
        let mut params = json!({ "profile": profile });
        if action == "stop" {
            params["force"] = json!(true);
        }
        let result = gateway
            .request(&format!("display.{action}"), params)
            .await
            .unwrap();
        let keys = ["running", "installed", "supported", "geometry", "placement", "blocker", "stopped"];
        let summary: serde_json::Map<String, Value> =
            keys.iter().filter_map(|k| result.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
        println!("{}", Value::Object(summary));
    }
}

/// Prints the server's gateway.ready payload (which wire dialect it speaks):
/// `cargo test ready_live -- --ignored --nocapture`.
#[cfg(test)]
mod ready_live {
    use super::*;
    use crate::backend::store;

    #[tokio::test]
    #[ignore]
    async fn ready_live() {
        let config = store::load(&store::config_path()).unwrap();
        let Some(id) = config.active_server.clone() else { return };
        let server = config.server(&id).unwrap().clone();
        let mut tokens = store::load_tokens(&server.id).expect("signed in");
        if tokens.expires_soon() {
            tokens = crate::backend::auth::refresh(&server.url, &tokens).await.ok().expect("refresh");
            store::save_tokens(&server.id, &tokens).unwrap();
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gateway = Gateway::connect(&server.url, &tokens.access_token, tx).await.unwrap();
        if let Some(Incoming::Event { kind, payload, .. }) = rx.recv().await {
            let keys: Vec<_> = payload.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
            println!("{kind} keys={keys:?} session_authority={}", payload.get("session_authority").unwrap_or(&Value::Null));
        }
    }
}

/// Calls one read-only RPC on the real server and prints a summary:
/// `HERMES_BOT_RPC=model.options HERMES_BOT_PARAMS='{"profile":"default"}' cargo test rpc_live -- --ignored --nocapture`.
/// `"$LIVE"` in the params is replaced by the profile's resumed Bot Chat.
#[cfg(test)]
mod rpc_live {
    use super::*;
    use crate::backend::store;

    #[tokio::test]
    #[ignore]
    async fn rpc_live() {
        let Ok(method) = std::env::var("HERMES_BOT_RPC") else { return };
        let params: Value = serde_json::from_str(&std::env::var("HERMES_BOT_PARAMS").unwrap_or("{}".into())).unwrap();
        let config = store::load(&store::config_path()).unwrap();
        let server = config.server(config.active_server.as_deref().unwrap()).unwrap().clone();
        let mut tokens = store::load_tokens(&server.id).expect("signed in");
        if tokens.expires_soon() {
            tokens = crate::backend::auth::refresh(&server.url, &tokens).await.ok().expect("refresh");
            store::save_tokens(&server.id, &tokens).unwrap();
        }
        let (tx, _rx) = mpsc::unbounded_channel();
        let gateway = Gateway::connect(&server.url, &tokens.access_token, tx).await.unwrap();
        // "$LIVE" in the params stands for the profile's Bot Chat, resumed first.
        let mut params = params;
        if params.to_string().contains("$LIVE") {
            let profile = params["profile"].as_str().unwrap_or("default").to_string();
            let stored = crate::backend::botchat::lookup(&gateway, &profile, None).await.unwrap().expect("bot chat");
            let live = crate::backend::botchat::resume(&gateway, &profile, &stored).await.unwrap().live;
            params = serde_json::from_str(&params.to_string().replace("$LIVE", &live)).unwrap();
        }
        let result = gateway.request(&method, params).await;
        let text = match result {
            Ok(v) => serde_json::to_string(&v).unwrap(),
            Err(e) => format!("ERROR {e}"),
        };
        // HERMES_BOT_FULL=1 prints the whole result instead of the first 3000 bytes.
        let limit = if std::env::var("HERMES_BOT_FULL").is_ok() { text.len() } else { text.len().min(3000) };
        println!("RESULT {}", &text[..limit]);
    }
}
