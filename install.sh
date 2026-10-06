#!/bin/sh
# Luma installer: downloads the release binary into ~/.local/bin (or $1).
# Nothing else is written: config/db are created by the binary itself on
# first run in ~/.config/luma/.
set -e

repo="https://github.com/sakurartro/luma"
bin="luma_rust"
dest="${1:-$HOME/.local/bin}"

mkdir -p "$dest"
arch=$(uname -m)
url="$repo/releases/latest/download/$bin-$arch"

echo "Downloading $url"
curl -fsSL "$url" -o "$dest/$bin"
chmod +x "$dest/$bin"

echo "Installed: $dest/$bin"
case ":$PATH:" in
    *":$dest:"*) ;;
    *) echo "Note: $dest is not in PATH. Add it: export PATH=\"$dest:\$PATH\"" ;;
esac
echo "Next: bind a hotkey in your DE settings to: $bin --toggle"
