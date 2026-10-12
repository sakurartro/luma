
<div align="center">

<img src="src/frontend/assets/file_0000000070c0820abb8763205e48e493.png" width="120" alt="Luma logo">

# Luma

**A fast, native launcher for Linux**

</div>

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/sakurartro/luma/main/install.sh | sh
```

The script downloads the binary into `~/.local/bin` (or a path of your choice: `... | sh -s /usr/local/bin`).

Luma does not grab global keys itself — on Wayland that is forbidden by
protocol design. It is launched via the command `luma_rust --toggle`, which
shows/hides the window of the running instance. Launching the binary again
while an instance is live also acts as a toggle. A cold start without the
flag opens the window right away.

## Hyprland

**Keybind:**

- **stock** — in `~/.config/hypr/hyprland.conf`:

  ```ini
  bind = SUPER, Space, exec, luma_rust --toggle
  ```

- **Omarchy (Lua config)** — in `~/.config/hypr/bindings.lua`:

  ```lua
  hl.bind("SUPER + SPACE", hl.dsp.exec_cmd("luma_rust --toggle"))
  ```

**Floating window** (Wayland apps cannot request floating themselves — the
compositor decides). Rule for the app id `luma-rust`:

- **stock** — in `~/.config/hypr/hyprland.conf`:

  ```ini
  windowrule = float, class:^(luma-rust)$
  ```

- **Omarchy (Lua config)** — in `~/.config/hypr/hyprland.lua` (Omarchy does
  not read `hyprland.conf`; the entry point is the Lua file):

  ```lua
  o.window("^(luma-rust)$", { float = true, center = true })
  ```

Reload the config afterwards: `hyprctl reload`.

## niri

**Keybind** — in `~/.config/niri/config.kdl`:

```kdl
binds {
    Mod+Space { spawn "luma_rust" "--toggle"; }
}
```

**Floating window** — in `~/.config/niri/config.kdl`:

```kdl
window-rule {
    match app-id="luma-rust"
    open-floating true
}
```

Restart niri afterwards to apply the config.

If `Super+Space` is already taken (e.g. input source switch), pick another
combo — e.g. `SUPER, D` / `Mod+D` — and use it consistently everywhere.

## Data

The app database lives at `$XDG_CONFIG_HOME/luma/main.sqlite3`
(`~/.config/luma/main.sqlite3` by default) and refreshes in the background
when `/usr/share/applications` changes.
