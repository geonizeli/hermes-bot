# Hermes Bot

A native Omarchy app for chatting with Hermes bots on remote Hermes servers.
Rust core, Qt Quick UI. See [PLAN.md](PLAN.md) for scope and milestones.

## Develop

```sh
bin/build     # release build into target/release/hermes-bot
bin/test      # cargo test, offscreen
bin/install   # build and install the pacman package (shows in the launcher)
```

Needs `rust`, `qt6-base`, `qt6-declarative`, `qt6-wayland` and `qt6-svg`.
