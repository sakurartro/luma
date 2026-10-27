<div align="center">

<img src="src/frontend/assets/file_0000000070c0820abb8763205e48e493.png" width="120" alt="Luma logo">

# Luma

**A Linux launcher that finds apps, files - and photos by what's in them.**

[![CI](https://github.com/sakurartro/luma/actions/workflows/ci.yml/badge.svg)](https://github.com/sakurartro/luma/actions/workflows/ci.yml)

![Luma demo](src/frontend/assets/gameplay.gif)

</div>

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/sakurartro/luma/main/install.sh | sh
```

The script downloads the binary into `~/.local/bin` (or a path of your choice: `... | sh -s /usr/local/bin`).

## Setup

Luma does not grab global keys itself — on Wayland that is forbidden by
protocol design. Bind a key to `luma --toggle`: it shows/hides the window
of the running instance; launching the binary again while an instance is live
acts the same. A cold start without the flag opens the window right away.

### Hyprland

**Keybind** — stock in `~/.config/hypr/hyprland.conf`; Omarchy (which does
not read `hyprland.conf`; the entry point is the Lua file) in
`~/.config/hypr/bindings.lua`:

```ini
bind = SUPER, Space, exec, luma --toggle
```

```lua
hl.bind("SUPER + SPACE", hl.dsp.exec_cmd("luma --toggle"))
```

**Floating window** for the app id `luma-rust` (Wayland apps cannot request
floating themselves — the compositor decides) — stock:

```ini
windowrule = float, class:^(luma-rust)$
```

Omarchy in `~/.config/hypr/hyprland.lua`:

```lua
o.window("^(luma-rust)$", { float = true, center = true })
```

Reload afterwards: `hyprctl reload`.

### niri

**Keybind** — in `~/.config/niri/config.kdl`:

```kdl
binds {
    Mod+Space { spawn "luma" "--toggle"; }
}
```

**Floating window** — same file:

```kdl
window-rule {
    match app-id="luma-rust"
    open-floating true
}
```

Restart niri to apply. If `Super+Space` is already taken (e.g. input source
switch), pick another combo — e.g. `SUPER, D` / `Mod+D` — and use it everywhere.

### GNOME / KDE (X11)

**GNOME** — Settings → Keyboard → Custom Shortcuts, command `luma --toggle`.
GNOME has no window rules; the window opens wherever the DE places it (force
floating with an extension, e.g. *Try*).

**KDE** — System Settings → Shortcuts → Add Custom Command `luma --toggle`.
Floating via Window Rules with Window class `luma-rust` (verify with `xprop`
that the class matches on X11).

Transparency on X11 requires a compositor — GNOME and KDE ship one by default.

## Features

- Fuzzy search of applications and files
- Separate views for apps, files, photos and settings (`Alt+1`…`Alt+4`)
- File type icons and cached photo thumbnails in the files grid
- Applications from all XDG dirs — system, `~/.local/share`, Flatpak, Snap; a user `.desktop` override shadows the system entry
- Launch ranking from usage stats (with time decay), pinned apps on top (right-click to pin/unpin)
- Category filter pills
- Calculator (`2 + 2`) and currency conversion (`100 usd to eur`)
- File index with a background watcher and scan progress
- Semantic photo search (CLIP) — opt-in; matching files show a `S(N%)` match-strength badge
- Text-in-image search (OCR) — opt-in; matching files show a `T(N%)` badge
- Power menu — shutdown, reboot, suspend, lock (power button in the bottom bar)
- Settings menu in the launcher (`Alt+4` / `Ctrl+,`) — edits `settings.json` for you
- Full name on hover when a label is truncated

## Usage

| Key | Action |
|-----|--------|
| `Enter` | Launch the selected (top) result |
| `↑` `↓` `←` `→` | Move the selection (left/right in the grid views) |
| `Alt+1` | Apps |
| `Alt+2` | Files |
| `Alt+3` | Photos |
| `Alt+4` / `Ctrl+,` | Settings |
| `Esc` | Hide the window |
| Right click | Pin / unpin an app |

## Configuration

Edit settings in the launcher (`Alt+4` / `Ctrl+,`) or by hand in
`~/.config/luma/settings.json`. Hand edits, `search_dirs`, `semantic` and `ocr`
apply after a restart (the menu offers a Restart button); everything else
applies at once. New options are appended with their defaults; a broken file
falls back to defaults and is kept.

| Option | Default | |
|--------|---------|--|
| `bg` `surface` `hover` `selected` `border` `accent` `text` `text_dim` | see below | Colors, `#RRGGBB`; an unparsable value renders black |
| `search_dirs` | `~/Documents` `~/Pictures` `~/Desktop` | Folders indexed for file search |
| `hide_patterns` | `["."]` | File name prefixes to hide (dotfiles) |
| `semantic` | `false` | Photo search by meaning (CLIP, English queries). Downloads ~154 MB of models, indexes photos in the background |
| `ocr` | `false` | Image search by the text on them (PaddleOCR, English). Downloads ~12 MB of models, runs after semantic indexing |
| `currency` | `true` | `100 usd to eur` via open.er-api.com, rates cached for an hour. `false` = no network; the calculator still works |

`semantic` and `ocr` cost nothing while off; turning them off keeps the models
and index on disk for later.

Where things live (`$XDG_CONFIG_HOME` / `$XDG_CACHE_HOME` respected):
`~/.config/luma/` — settings, `main.sqlite3`, `models/`;
`~/.cache/luma/thumbs` — photo thumbnails.
Apps are read from `applications/` in `$XDG_DATA_HOME` and every
`$XDG_DATA_DIRS` entry, refreshed when those dirs change.

Full example (the defaults):

```json
{
  "bg": "#0E120B", "surface": "#151B10", "hover": "#1E2716", "selected": "#28331C",
  "border": "#2C3620", "accent": "#A9B665", "text": "#E4E7D0", "text_dim": "#7E8A6C",
  "search_dirs": ["~/Documents", "~/Pictures", "~/Desktop"],
  "hide_patterns": ["."],
  "semantic": false,
  "ocr": false,
  "currency": true
}
```

### Palettes

Cold dark:

```json
{"bg": "#0D1117", "surface": "#161B22", "hover": "#1C2431", "selected": "#24304A", "border": "#2D3748", "accent": "#58A6FF", "text": "#E6EDF3", "text_dim": "#8B949E"}
```

Warm (gruvbox-like):

```json
{"bg": "#282828", "surface": "#32302F", "hover": "#3C3836", "selected": "#504945", "border": "#665C54", "accent": "#D8A657", "text": "#FBF1C7", "text_dim": "#A89984"}
```

Light:

```json
{"bg": "#FBF7EF", "surface": "#F1EBDD", "hover": "#E9E1CE", "selected": "#DFE4CB", "border": "#D3CBB2", "accent": "#5B7A3A", "text": "#2E3324", "text_dim": "#8A8B76"}
```

## Requirements

- Linux (glibc 2.39+ — Ubuntu 24.04, Fedora 40+, Arch; the ONNX runtime of the semantic search sets this floor)
- The prebuilt binary links at runtime against libfontconfig, libGL (mesa) and libxkbcommon
- Power menu uses systemd: `systemctl poweroff|reboot|suspend`; lock is `loginctl lock-session`, so a locker must listen for it (e.g. hypridle with `lock_cmd`)

## Build from source

```sh
cargo build --release
```

Dev packages (Debian/Ubuntu naming): `libxkbcommon-dev libfontconfig1-dev libgl1-mesa-dev libxcb1-dev`. Binary: `target/release/luma`.

## Troubleshooting

- Reset — delete `~/.config/luma/` (settings and database are recreated; models are downloaded again if enabled)
- Wrong or stale thumbnails — delete `~/.cache/luma/thumbs` (regenerated on demand)
- Stuck hotkey — kill the process or remove `$XDG_RUNTIME_DIR/luma-rust.sock` (or `/tmp/luma-rust-<uid>.sock`)
- Logs go to stderr — run Luma from a terminal to see them

## Uninstall

```sh
rm -f ~/.local/bin/luma ~/.local/bin/luma_rust && rm -rf ~/.config/luma ~/.cache/luma
```
