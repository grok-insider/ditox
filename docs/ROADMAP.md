# Ditox

Clipboard manager in Rust: the `ditoxd` daemon and the `ditox` window (Rune).

## Shipped

- SQLite history (schema 5), FTS and fuzzy search, content-addressed images, watcher, Hyprland paste-back (0.57 Lua dispatch and earlier).
- JSON-RPC on `$XDG_RUNTIME_DIR/ditox/rpc.sock`, with shared types in `ditox-rpc`.
- The list, after Ditto: clips with their first lines or a small picture, numbers for the first nine, a search field under them and one menu. Paste, paste as text, copy only, paste by number, see a clip whole, open a link, pin, delete with undo, narrow to text, images or pinned, pause and resume, daemon-down state, light and dark. On Hyprland it keeps the keyboard whatever the pointer is over, goes away with Escape, its own key or a click outside, and opens where and as large as it was left, always wholly on the screen.
- With the hypr-overlay plugin, an image seen whole is drawn large on the screen by the compositor, beside or clear of the list, and follows the selection. Without it, in the window.
- The options, a window of their own (`ditox options`): history, paste, privacy, the keys, and where things are. The daemon checks what is saved, writes `config.toml` and lives by it at once (`config.get`, `config.set`).
- Paste into a terminal with its own shortcut. Hyprland is asked through its socket.

## Memory and speed

Measured on the station (NVIDIA, Hyprland 0.57.1, driver 620.70) with a copy of a real history of 1000 clips, 205 of them images, in a headless compositor that renders on the same GPU. Resident memory of the list:

| | |
|---|---|
| Open, five rows with a picture in view | 111 MB |
| Narrowed to images, ten in view | 115 MB |
| After paging through 96 image rows | 117 MB |
| After seeing 25 images whole | 129 MB |

About 80 MB of it is the NVIDIA driver's libraries, which every program that draws with the GPU maps and shares. The window's own binary is about 23 MB, and what it allocates about 20 MB at open. With the preview pane of 0.2's first window the same history opened at 112 MB and reached 135 MB after 14 screenshots.

- The rows' pictures are on screen within 0.35 s of starting the program. They are fetched one at a time, for the rows in view and six past them, and made at most 240 by 60 points.
- The window does not start `hyprctl` before it opens (20 to 50 ms); Hyprland's socket answers in 0.1 ms.
- A paste asks Hyprland four times through the socket, in under a millisecond together.
- Under a compositor that renders in software the first try fails and the window starts again with every driver: about 230 MB. The same happens to the driver's share without an X server (no `DISPLAY`): the NVIDIA driver then loads its EGL and Mesa's libraries, about 220 MB.

## Distribution

Nix flake. CI builds `x86_64-linux` and `aarch64-linux` and pushes to `https://cache.fabrials.com/fabrials`.
