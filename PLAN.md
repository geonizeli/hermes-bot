# Hermes Desktop — requirements and plan

A native Omarchy app for chatting with Hermes bots on remote Hermes servers,
as close as possible to the Grok chat experience.

## Requirements

### Gathered
- **Purpose:** talk to Hermes bots. Nothing else (no config, cron, kanban or
  log screens from the web dashboard).
- **Fully native** client; no embedded web view. **Rust wherever possible**:
  a Rust core with a Qt Quick (QML) UI.
- **Connections:** remote servers over HTTPS, signing in with OAuth or a
  password. The app keeps its own list and does not read Hermes' files.
- **Look and feel:** as close to Grok as possible.
- **Setup:** Omarchy on Hyprland, one 4K display at 1.33× scale. Built the
  way Omarchy's own apps are (see the omarchy-app conventions): one binary,
  live theme accent and window rounding, keyboard first, installed as a
  pacman package so it shows in the launcher.

### Derived from the Hermes source
- **Chat protocol:** JSON-RPC 2.0 over `wss://<host>/api/ws`
  (`tui_gateway`), the same one the official desktop app uses. The contract
  is `apps/shared/src/gateway-contract.openrpc.json`.
- **Auth, OAuth:** RFC 8252 brokered PKCE. The app listens on
  `http://127.0.0.1:<port>/callback`, opens the system browser at
  `/auth/native/authorize`, exchanges the code at `/auth/native/token`, and
  refreshes at `/auth/native/refresh` 60 s before `expires_at`.
- **Auth, password:** a native form posts `/auth/password-login` through the
  same brokered PKCE flow, so no browser is needed and the app ends up with
  the same bearer and refresh tokens.
- **WebSocket auth:** `POST /api/auth/ws-ticket` with the bearer token, then
  connect with `?ticket=` (single use, 30 s).
- **Discovery:** `GET /api/health` and `GET /api/status` (`auth_flows`,
  `auth_providers`, `profiles`, version), then `GET /api/auth/providers`.
- **Bots are Hermes profiles:** listed with `profiles.list`; every call
  carries `profile`.

## Feature set (v1)

| Area | What it does |
|---|---|
| Servers | Add a server by URL; the app probes it and shows its providers. Sign in with OAuth (system browser) or a password (native form). Tokens go to the Secret Service keyring (`keyring` crate); the server list lives in `~/.config/hermes-desktop/servers.json`. |
| Bots | Bot picker for the server's profiles, showing avatar, display name and description. The last bot used is remembered. |
| Sidebar | Grok-style: new chat, search, history grouped by Today, Yesterday, Previous 7 days and Older, plus rename, delete and pin. Collapsible. |
| Empty state | Centered greeting and one large rounded composer. |
| Conversation | User turns in bubbles on the right; assistant turns full width as Markdown with code blocks. Streaming with a typing cursor. A collapsible "Thought for N s" block for reasoning. Compact tool-call chips that expand to show args and result. Copy and regenerate under each answer. |
| Composer | Multi-line input; Enter sends and Shift+Enter adds a newline. Model picker and reasoning-effort pill. Attach images and files through the xdg portal or by pasting. Stop button while streaming. |
| Agent requests | Approval cards (once / session / always / deny) and clarify questions rendered inline. |
| Resilience | Heartbeat, auto-reconnect, replay of missed events with `session.events.since`, and clear in-window errors (offline, signed out, server too old). |
| Omarchy | Live accent and rounding, follows the Omarchy language, HiDPI-crisp. Shortcuts: Ctrl+N new chat, Ctrl+K search, Ctrl+B sidebar, Ctrl+Shift+O bots, Esc stop, Ctrl+Q quit, F1 shortcuts overlay. |

Plain-letter shortcuts from the Omarchy convention (`Q`, `?`, `Space`) are
mapped to Ctrl/F1 variants, because this is a typing app and those keys
belong to the composer.

### Out of scope for v1
SSH tunnels, local `hermes dashboard`, the web dashboard's admin screens,
voice.

## Architecture

Rust owns all logic; QML owns layout and interaction. They are bridged with
[cxx-qt](https://github.com/KDAB/cxx-qt): Rust `QObject`s expose properties,
signals and invokables to QML, and Qt's event loop drives the UI while a
tokio runtime runs the network work and posts results back to the Qt thread.

```text
Cargo.toml              one crate, binary `hermes-desktop`
build.rs                cxx-qt-build: bridges, QML module, i18n resources
src/main.rs             app setup: QGuiApplication, Material style, desktop file name
src/bridge/             cxx-qt QObjects handed to QML
  app.rs                servers, active server and bot, sign-in state, errors
  sessions.rs           sidebar history list model
  chat.rs               conversation list model; applies streaming events
  bots.rs               profiles of the active server
  theme.rs              Omarchy accent + Hyprland rounding, watched live
  translator.rs         Omarchy language, JSON catalogs, plural rules
src/core/               plain Rust, no Qt, unit-tested
  store.rs              servers.json (atomic writes) + keyring tokens
  auth.rs               discovery, PKCE loopback flow, password broker flow, refresh
  gateway.rs            WebSocket JSON-RPC: tickets, requests, events, heartbeat, replay
  protocol.rs           serde types for the methods and events we use
  transcript.rs         event stream -> turns (text, reasoning, tools) state machine
qml/                    Main, Sidebar, ChatView, MessageBubble, Composer,
                        ToolChip, ReasoningBlock, ApprovalCard, BotPicker,
                        ServerDialog, ShortcutsOverlay
i18n/<code>.json        catalogs compiled into the binary
bin/build bin/test bin/install
pkgbuild/PKGBUILD .desktop .svg
```

Crates: `cxx-qt`/`cxx-qt-lib`, `tokio`, `reqwest` (rustls), `tokio-tungstenite`,
`serde`/`serde_json`, `keyring` (Secret Service), `sha2`/`base64`/`rand` for
PKCE, `notify` for the theme and language watchers, `open` for the system
browser. Tests: `cargo test` for `core/` against an in-process fake Hermes
server (HTTP + WebSocket), plus an offscreen QML harness for the UI.

Qt modules: core, gui, qml, quick, quickcontrols2, svg, wayland. File dialogs
go through the xdg-desktop-portal (`ashpd` crate).

## Milestones (one commit or more each)

1. Scaffold: cargo + cxx-qt app showing a themed window; build, test and
   install scripts, PKGBUILD, theme and translator ported to Rust.
2. Server store, discovery and both sign-in flows, with keyring storage and
   refresh.
3. Gateway client and transcript state machine, tested against a fake server.
4. Sidebar history and bot picker.
5. Streaming conversation view (Markdown, reasoning, tools, stop).
6. Composer extras: model and effort pickers, attachments.
7. Approval and clarify cards, reconnect and replay.
8. Polish: shortcuts overlay, i18n, screenshot, README, `bin/install`.
