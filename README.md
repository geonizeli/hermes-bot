# Hermes Bot

A native Linux app for chatting with Hermes bots on remote Hermes servers.
It runs on any desktop (GNOME, KDE, Sway, Hyprland…) and fits in on Omarchy,
where it can follow the live Omarchy theme.
Rust core, GPUI interface. See [PLAN.md](PLAN.md) for scope and milestones.

## Develop

```sh
bin/build                 # release binary into target/release/hermes-bot
bin/test                  # cargo test

bin/build-for-archlinux   # pacman package under pkgbuild/
bin/build-for-appimage    # portable AppImage in dist/ (curl/wget; Vulkan/GPU from host)
bin/build-for-macos       # .app + binary in macos-dist/

bin/install-for-archlinux # build and install the pacman package (launcher)
bin/install-for-appimage  # build AppImage and install to ~/.local/bin/hermes-bot
bin/install-for-macos     # build and install to ~/Applications + ~/bin
```

Needs `rust`. At run time: Vulkan (`vulkan-icd-loader` and a driver),
`libxkbcommon`, `wayland` (or X11), `fontconfig`.

Optional, used when present:
- a Secret Service keyring (GNOME Keyring, KWallet, KeePassXC). Bearer tokens
  are stored there, not in `servers.json`. On startup any legacy plain
  `tokens.json` is moved into the keyring and removed. Without a keyring,
  sign-ins last until the app quits;
- `xdg-desktop-portal` with a backend for the file chooser, or `zenity` /
  `kdialog`; files can always be dropped or pasted into the chat.

## Themes

Built-in dark and light themes, or *Match system* to follow the desktop's
light/dark setting. On Omarchy the live Omarchy theme is the default and stays
one of the choices. Pick one from the server menu (*Theme…*) or with
`/skin dark|light|system|omarchy`.

Under Hyprland the corners follow its rounding. Where the compositor leaves
window decorations to apps (GNOME on Wayland), the window draws its own title
bar; `HERMES_BOT_DECORATIONS=client` asks for it elsewhere too.
