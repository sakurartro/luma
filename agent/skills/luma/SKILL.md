---
name: luma
description: Find photos and images on this computer by what is in them (e.g. "the photo of a river in fog", "a picture of my cat on the windowsill") using the running Luma launcher's semantic (CLIP) index. Use instead of find/grep/ls when the user asks to find an image, photo or picture by its content.
---

# Luma: search images by meaning

Luma is a launcher that keeps a CLIP index of the user's photos in memory.
Asking it is a sub-second call; walking the home folder with `find` is not,
and `find` cannot match by content anyway.

## Search

```bash
luma query clip "river in fog"
luma query clip --limit 10 "cat sitting on a windowsill"
```

- **Write the query in English.** The CLIP model only understands English;
  translate the user's request first ("кот на подоконнике" → "cat on a windowsill").
- Describe what is visible, not file metadata: "red car parked at night",
  not "photo from last summer" or "IMG_2034".
- `--limit N` (default 5). Luma returns at most 32 hits whatever the limit.

Output is one JSON line on stdout, best match first:

```json
[{"path":"/home/user/Pictures/fog.jpg","score":71},{"path":"/home/user/Pictures/lake.png","score":40}]
```

`score` is match strength, 0–100. It is relative: compare hits within one
answer, don't treat a number as a fixed threshold. `[]` means nothing matched
well enough — rephrase the query (more concrete, fewer words) before giving up.

## Check before you answer

CLIP finds similar-looking images, not guaranteed matches. Open the top few
hits (read the image files) and confirm they show what the user asked for
before presenting them. If none fit, say so and try another phrasing.

## What is searched

Only images inside the folders listed in `search_dirs` of
`~/.config/luma/settings.json` (default `~/Documents`, `~/Pictures`, `~/Desktop`).
Files whose name starts with a prefix in `hide_patterns` (default `.`) are
never returned. An image outside these folders won't be found — fall back to
normal file tools for it, and tell the user they can add the folder to
`search_dirs` (applies after restarting Luma).

## Errors

Errors go to stderr with exit code 1. Pass the fix on to the user; don't
edit their settings or start Luma without asking.

| Message | Meaning / fix |
|---|---|
| `Luma is not running` | Start the Luma launcher (`luma`), then retry. |
| `semantic search is not available: ...` | Either `"semantic": true` is not set in `~/.config/luma/settings.json` (set it, restart Luma; it downloads ~154 MB of models and indexes photos in the background), or indexing is still running — wait and retry. |
| `agent API is disabled: ...` | The user must set `"agent_api": true` in `~/.config/luma/settings.json` and restart Luma. |
| `unexpected answer from Luma (an older version still running?)` | An old Luma daemon is running; restart Luma. |
| `luma: command not found` | Luma isn't installed or not on `PATH`; use normal file tools. |
