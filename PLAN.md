# Hermes Bot — requirements and plan

A native Linux app for chatting with Hermes bots on remote Hermes servers,
as close as possible to the Grok chat experience. Born on Omarchy, it now
runs on any Linux desktop and keeps Omarchy's theme as an option.

## Requirements

### Gathered
- **Purpose:** talk to Hermes bots. Nothing else (no config, cron, kanban or
  log screens from the web dashboard).
- **Fully native** client; no embedded web view. **Rust wherever possible**:
  a Rust core with a [GPUI](https://gpui.rs) UI (through gpui-kit, which adds
  styled inputs, scrollbars and tooltips on top of GPUI).
- **Connections:** remote servers over HTTPS, signing in with OAuth or a
  password. The app keeps its own list and does not read Hermes' files.
- **Look and feel:** function first, then the Grok-like look on built-in
  dark/light themes or the live Omarchy palette.
- **Platforms:** Linux only, any desktop. Omarchy/Hyprland integration (live
  theme, rounding) is used when present, never required. Developed on
  Omarchy on Hyprland, one 4K display at 1.33× scale; one binary, installed
  as a pacman package so it shows in the launcher.
- **Desktop independence (November 2026):** built-in themes that can follow
  the system's light/dark preference, Omarchy as one theme choice, a drawn
  title bar where the compositor leaves decorations to apps, and fallbacks
  without a keyring (sign-in kept in memory) or a file-chooser portal
  (zenity, kdialog).

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
| Servers | Add a server by URL; the app probes it and shows its providers. Sign in with OAuth (system browser) or a password (native form). Tokens go to the Secret Service keyring (`keyring` crate), or stay in memory for the run when there's none; the server list lives in `~/.config/hermes-bot/servers.json`. |
| Bots | Bot picker for the server's profiles, showing avatar, display name and description. The last bot used is remembered. Edit profile (right-click a bot, or click its name in the header): title, description and avatar (classic shapes, blobatar faces, colors, upload, generate, petdex pet), as the official desktop's dialog minus Advanced. |
| Sidebar | Grok-style: new chat, search, history grouped by Today, Yesterday, Previous 7 days and Older, plus rename, delete and pin. Collapsible. |
| Empty state | Centered greeting and one large rounded composer. |
| Conversation | User turns in bubbles on the right; assistant turns full width as Markdown with code blocks. Streaming with a typing cursor. A collapsible "Thought for N s" block for reasoning. Compact tool-call chips that expand to show args and result. Copy and regenerate under each answer. All chat text (your messages, answers, reasoning, code, tool output) is selectable across messages; Ctrl+C copies, and links still open on a click. **Edit** on any of your messages opens it in place; sending replaces it and everything after it (`prompt.submit` with `truncate_before_row_id` + `confirm_truncate`, `confirm_deep_truncate` when later messages go, `confirm_empty_truncate` for the first), keeps its attachments (`image.attach` by server path, `@file:` refs), and rebinds the kept rows' ids from `survivor_row_id_map`. |
| Composer | Multi-line input; Enter sends and Shift+Enter adds a newline. Model picker and reasoning-effort pill. Attach images and files through the xdg portal (zenity or kdialog without one) or by dropping and pasting. Stop button while streaming. While the bot answers, Enter steers it and **Queue** sends after it: queued messages wait in a panel above the message box (edit, remove, steer now, send now), go out in order with `prompt.submit {queued: true}` as each answer ends (also for bots whose chat isn't open), and pause after a stop, failure or lost connection until resumed. Kept in memory. |
| Slash commands | The chat's command set comes from the server (`commands.catalog` per session: commands, aliases, subcommands, skills, and which ones a desktop client may use). A menu above the composer filters it as you type `/` (↑↓, Tab or Enter to complete, Esc to close). Commands run on the server (`slash.exec`, then `command.dispatch`); output shows as a monospace card, skills send their message while the bubble shows the invocation. Handled here: `/stop`, `/steer` while answering, `/model` (picker), `/reasoning <level>`, `/help`, `/skin` (theme), `/queue`, and `/compress` — which `/new`, `/reset` and `/clear` map to, as in the official Bot Mode. The official desktop's other client actions (`/branch`, `/title`…) say they aren't available. |
| Agent requests | Approval cards (once / session / always / deny) and clarify questions rendered inline. |
| Resilience | Heartbeat, auto-reconnect, replay of missed events with `session.events.since`, and clear in-window errors (offline, signed out, server too old). |
| Look | Built-in dark and light themes or Match system; the live Omarchy theme where installed. Hyprland's rounding under Hyprland. A drawn title bar where the compositor doesn't draw one. HiDPI-crisp. |

### Out of scope for v1
SSH tunnels, local `hermes dashboard`, the web dashboard's admin screens,
voice, custom keyboard shortcuts (revisit later).

## Architecture

Everything is Rust. GPUI draws the window; `ui::state::AppState` is one
entity holding the app's state and actions, and the views read it and call
its methods. Network work runs on a tokio runtime; when it's done it queues a
closure back onto the UI thread (`ui::queue::Queue`), which runs with the
entity and its context. One-off happenings (a probed server, a question from
the bot) reach the views as `AppEvent`s.

```text
Cargo.toml              one crate, binary `hermes-bot`
src/main.rs             app setup: window, app_id (matches hermes-bot.desktop)
src/backend/            plain Rust, no UI, unit-tested
  store.rs              servers.json (atomic writes, UI prefs) + keyring tokens
  auth.rs               discovery, PKCE loopback flow, password broker flow, refresh
  gateway.rs            WebSocket JSON-RPC: requests, events, server requests
  botchat.rs            each bot's hidden "Bot Chat" session
  history.rs            REST transcript -> conversation rows
  markdown.rs           Markdown -> styled paragraphs, code spans, tables, alerts
  slash.rs              server-driven slash commands (commands.catalog)
  rfb.rs                VNC client for a bot's screen
  theme.rs              built-in themes, theme choice, Omarchy palette +
                        Hyprland rounding (watched live)
  face/, botedit.rs     bot avatars and the Edit Bot logic
src/ui/                 GPUI views
  state/                AppState: connection, roster, transcript model,
                        commands, editor, message edits, queue
  queue.rs              tokio -> UI thread hand-off
  theme.rs              theme state and palette as globals; syncs the
                        component theme
  root.rs               window: sidebar, menus, dialogs
  chat.rs               header, conversation list, side panel, file drops
  composer.rs           message box, command menu, model/effort pickers
  markdown_view.rs      answer blocks, images, copy buttons
  selectable.rs         selectable rich text (window-wide selection, links)
  tool_card.rs          tool call chips and to-do lists
  side_panel.rs         scheduled jobs and screen controls
  screen.rs             live screen view and input forwarding
  dialogs.rs            servers, bot questions, expensive-model confirmation
  edit_bot.rs           Edit Bot dialog and pet picker
bin/build bin/test bin/build-for-* bin/install-for-*
pkgbuild/PKGBUILD .desktop .svg
```

Crates: `gpui-kit` (GPUI), `tokio`, `reqwest` (rustls), `tokio-tungstenite`,
`serde`/`serde_json`, `keyring` (Secret Service, in-memory fallback), `sha2`/`base64`/`rand` for
PKCE, `notify` for the theme watchers, `open` for the system browser,
`pulldown-cmark` + `syntect` for answers, `chrono` for local times. File
dialogs go through the xdg-desktop-portal (`ashpd` crate), falling back to
zenity or kdialog. Tests: `cargo test`.

The app was first built with Qt Quick through cxx-qt and moved to GPUI in
November 2026; the milestones below date from then.

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
8. Polish: i18n, screenshot, README, `bin/install-for-*`.
