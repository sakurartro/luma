# Luma

Десктопный лаунчер приложений для Linux (X11/Wayland).

## Установка

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

## Данные

БД приложений: `$XDG_CONFIG_HOME/luma/main.sqlite3` (по умолчанию
`~/.config/luma/main.sqlite3`), обновляется в фоне при изменении
`/usr/share/applications`.
