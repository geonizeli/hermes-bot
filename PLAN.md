# Hermes Desktop — requirements and plan

A native Omarchy app for chatting with Hermes bots on remote Hermes servers,
as close as possible to the Grok chat experience.

## Requirements

### Gathered
- **Purpose:** talk to Hermes bots. Nothing else (no config, cron, kanban or
  log screens from the web dashboard).
- **Fully native** C++ / Qt Quick client; no embedded web view.
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
| Servers | Add a server by URL; the app probes it and shows its providers. Sign in with OAuth (system browser) or a password (native form). Tokens go to the Secret Service keyring through libsecret; the server list lives in `~/.config/hermes-desktop/servers.json`. |
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

```text
src/main.cpp            app setup
src/serverstore.*       servers.json (atomic saves) + keyring tokens (libsecret)
src/auth.*              PKCE loopback flow, password broker flow, refresh
src/gatewayclient.*     WebSocket JSON-RPC: tickets, requests, events, heartbeat, replay
src/sessionmodel.*      QAbstractListModel of the sidebar history
src/chatmodel.*         QAbstractListModel of turns; applies streaming events
src/botmodel.*          profiles of the active server
src/theme.* translator.*  Omarchy templates
src/qml/                Main, Sidebar, ChatView, MessageBubble, Composer,
                        ToolChip, ReasoningBlock, ApprovalCard, BotPicker,
                        ServerDialog, ShortcutsOverlay
tests/                  Qt Test against a fake in-process HTTP + WebSocket server
```

Qt modules: core, gui, qml, quick, quickcontrols2, network, websockets, dbus,
svg. Linked library: libsecret-1.

## Milestones (one commit or more each)

1. Scaffold from the Omarchy templates: build, test and install scripts,
   PKGBUILD, theme, translator.
2. Server store, discovery and both sign-in flows, with keyring storage and
   refresh.
3. Gateway client with tests against a fake server.
4. Sidebar history and bot picker.
5. Streaming conversation view (Markdown, reasoning, tools, stop).
6. Composer extras: model and effort pickers, attachments.
7. Approval and clarify cards, reconnect and replay.
8. Polish: shortcuts overlay, i18n, screenshot, README, `bin/install`.
