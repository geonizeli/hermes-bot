# Hermes Bot

A native Omarchy app for chatting with Hermes bots on remote Hermes servers.
Rust core, GPUI interface. See [PLAN.md](PLAN.md) for scope and milestones.

## Develop

```sh
bin/build     # release build into target/release/hermes-bot
bin/test      # cargo test
bin/install   # build and install the pacman package (shows in the launcher)
```

Needs `rust`. At run time: Vulkan (`vulkan-icd-loader` and a driver),
`libxkbcommon`, `wayland`, `fontconfig` and `xdg-desktop-portal`.
