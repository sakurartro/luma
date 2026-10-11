# Luma

Десктопный лаунчер приложений для Linux (X11/Wayland).

## Установка

```sh
curl -fsSL https://raw.githubusercontent.com/sakurartro/luma/main/install.sh | sh
```

Скрипт скачает бинарник в `~/.local/bin` (или в переданный путь: `... | sh -s /usr/local/bin`).

Альтернатива для разработчиков:

```sh
cargo install --path .
```

## Горячая клавиша

Luma не перехватывает глобальные клавиши сам: на Wayland это запрещено дизайном
протокола. Привяжите `luma_rust --toggle` в настройках вашего DE/композитора:

- **Hyprland**: `bind = SUPER, Space, exec, luma_rust --toggle`
- **sway**: `bindsym $mod+space exec luma_rust --toggle`
- **KDE/GNOME**: Системные настройки → Комбинации клавиш → добавить команду `luma_rust --toggle`

Флаг `--toggle` показывает/прячет окно уже запущенного экземпляра; повторный
запуск без него тоже поднимет окно. Запуск без флага при холодном старте
открывает окно сразу.

## Floating window (Wayland)

Wayland apps cannot request floating themselves — the compositor decides.
If Luma opens as a tiled window, add a rule for its app id (`luma-rust`):

**Hyprland** — in `~/.config/hypr/hyprland.conf`:

```ini
windowrule = float, class:^(luma-rust)$
```

**niri** — in `~/.config/niri/config.kdl`:

```kdl
window-rule {
    match app-id="luma-rust"
    open-floating true
}
```

Reload the config afterwards (Hyprland: `hyprctl reload`, niri: restart it) —
Luma will now open as a floating overlay.

## Default keybind (Super + Space)

The install above only makes `--toggle` available — nothing calls it yet.
Bind it to open Luma with the same key everyone knows from other launchers:

**Hyprland** — in `~/.config/hypr/hyprland.conf`:

```ini
bind = SUPER, Space, exec, luma_rust --toggle
```

**niri** — in `~/.config/niri/config.kdl`:

```kdl
binds {
    Mod+Space { spawn "luma_rust" "--toggle"; }
}
```

If `Super+Space` is already taken (e.g. input source switch), pick another
combo — e.g. `SUPER, D` / `Mod+D` — and use it consistently everywhere.

## Данные

БД приложений: `$XDG_CONFIG_HOME/luma/main.sqlite3` (по умолчанию
`~/.config/luma/main.sqlite3`), обновляется в фоне при изменении
`/usr/share/applications`.
