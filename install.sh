#!/bin/sh
# Luma installer: downloads the release binary into ~/.local/bin (or $1).
# Nothing else is written: config/db are created by the binary itself on
# first run in ~/.config/luma/.
set -e

repo="https://github.com/sakurartro/luma"
bin="luma"
dest="${1:-$HOME/.local/bin}"

if ! command -v curl >/dev/null 2>&1; then
    echo "Error: curl is required to download the release." >&2
    exit 1
fi
if ! command -v sha256sum >/dev/null 2>&1; then
    echo "Error: sha256sum is required to verify the release." >&2
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

# Resolve "latest" to a tag once, so the binary and its checksum come from the
# same release even if a new one is published mid-install.
tag=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$repo/releases/latest") || tag=""
tag="${tag##*/}"
case "$tag" in
    v*) ;;
    *)
        echo "Error: cannot resolve the latest release (check your network; releases are at $repo/releases)." >&2
        exit 1
        ;;
esac
url="$repo/releases/download/$tag/$bin-$arch"

# Temp file + rename: writing over a running binary fails with ETXTBSY, and a
# broken download must not destroy the installed one.
tmp="$dest/.$bin.tmp.$$"
trap 'rm -f "$tmp"' EXIT

echo "Downloading $url"
if ! curl -fsSL "$url" -o "$tmp"; then
    echo "Error: download failed (check your network; the release exists at $repo/releases)." >&2
    exit 1
fi
# Same-release checksum: catches corrupted/truncated downloads, not a
# compromised publisher account.
if ! sum=$(curl -fsSL "$url.sha256"); then
    echo "Error: cannot download $url.sha256 to verify the binary." >&2
    exit 1
fi
expected="${sum%% *}"
actual=$(sha256sum "$tmp")
actual="${actual%% *}"
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    echo "Error: checksum mismatch for $url (expected $expected, got $actual); nothing installed." >&2
    exit 1
fi
chmod +x "$tmp"
mv -f "$tmp" "$dest/$bin"
# Installs from before the rename: hotkeys still call luma_rust, keep it as a link.
if [ -e "$dest/luma_rust" ]; then
    ln -sf "$bin" "$dest/luma_rust"
fi

echo "Installed: $dest/$bin"
if pgrep -x "$bin" >/dev/null 2>&1 || pgrep -x luma_rust >/dev/null 2>&1; then
    echo "Note: an older Luma is still running; restart it: pkill -x $bin; pkill -x luma_rust; $bin &"
fi
case ":$PATH:" in
    *":$dest:"*) ;;
    *) echo "Note: $dest is not in PATH. Add it: export PATH=\"$dest:\$PATH\"" ;;
esac
echo "Next: bind a hotkey in your DE settings to: $bin --toggle"
