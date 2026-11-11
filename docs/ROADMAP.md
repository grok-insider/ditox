# Ditox

Clipboard manager in Rust: the `ditoxd` daemon and the `ditox` window (Rune).

## Shipped

- SQLite history (schema 5), FTS and fuzzy search, content-addressed images, watcher, Hyprland paste-back (0.57 Lua dispatch and earlier).
- JSON-RPC on `$XDG_RUNTIME_DIR/ditox/rpc.sock`, with shared types in `ditox-rpc`.
- Window in the Fabrials style: search with its finds marked, text/image/pin filters, day headings, rows that show a link or a colour, preview, paste, paste as text, copy only, open a link, pin, delete with undo, a sheet of every key, pause and resume, daemon-down state, light and dark.
- Paste into a terminal with its own shortcut. Hyprland is asked through its socket.

## Memory and speed

Measured on the station (NVIDIA, Hyprland 0.57.1, driver 620.70) with a copy of a real history of 1000 clips, in a headless compositor that renders on the same GPU. Resident memory of the window:

| | Before | Now |
|---|---|---|
| Open, a text clip selected | 150 MB | 112 MB |
| Open, a 4K screenshot selected | 195 MB | 118 MB |
| After looking at 25 images | 210 MB | 123 MB |
| After looking at 14 screenshots, 1080p to 4K | 366 MB | 135 MB |

About 80 MB of what is left is the NVIDIA driver's libraries, which every program that draws with the GPU maps and shares. The window's own binary is about 23 MB of it, and what it allocates about 20 MB at open.

- First frame of a headless run: 0.13 s before, 0.11 s now. The window also no longer starts `hyprctl` before it opens (20 to 50 ms); the socket answers in 0.1 ms.
- A paste asked Hyprland three times through `hyprctl`; it now asks four times through the socket, in under a millisecond together.
- Under a compositor that renders in software the first try fails and the window starts again with every driver: about 230 MB, as before.

## Distribution

Nix flake. CI builds `x86_64-linux` and `aarch64-linux` and pushes to `https://cache.fabrials.com/fabrials`.
