#!/bin/sh
# Luma installer: downloads the release binary into ~/.local/bin (or $1).
# Nothing else is written: config/db are created by the binary itself on
# first run in ~/.config/luma/.
set -e

repo="https://github.com/sakurartro/luma"
bin="luma_rust"
dest="${1:-$HOME/.local/bin}"

if ! command -v curl >/dev/null 2>&1; then
    echo "Error: curl is required to download the release." >&2
    exit 1
fi

arch=$(uname -m)
if [ "$arch" != "x86_64" ]; then
    echo "Error: no build is published for architecture '$arch' yet (only x86_64)." >&2
    exit 1
fi

if ! mkdir -p "$dest" || [ ! -w "$dest" ]; then
    echo "Error: cannot create or write to '$dest' (check permissions, or try a user-owned dir like ~/.local/bin)." >&2
    exit 1
fi

url="$repo/releases/latest/download/$bin-$arch"

# Temp file + rename: writing over a running binary fails with ETXTBSY, and a
# broken download must not destroy the installed one.
tmp="$dest/.$bin.tmp.$$"
trap 'rm -f "$tmp"' EXIT

echo "Downloading $url"
if ! curl -fsSL "$url" -o "$tmp"; then
    echo "Error: download failed (check your network; the release exists at $repo/releases)." >&2
    exit 1
fi
chmod +x "$tmp"
mv -f "$tmp" "$dest/$bin"

echo "Installed: $dest/$bin"
if pgrep -x "$bin" >/dev/null 2>&1; then
    echo "Note: an older Luma is still running; restart it: pkill -x $bin && $bin &"
fi
case ":$PATH:" in
    *":$dest:"*) ;;
    *) echo "Note: $dest is not in PATH. Add it: export PATH=\"$dest:\$PATH\"" ;;
esac
echo "Next: bind a hotkey in your DE settings to: $bin --toggle"
