# Ditox

Ditox is a clipboard manager for Linux/Wayland (first target: Hyprland). It is written in Rust.

- **`ditoxd`** owns the clipboard, the SQLite history, image files, and paste-back.
- **`ditox`** is the window, built with [Rune](https://github.com/anomalyco/rune): search, preview, pin, delete, and paste.
- They talk JSON-RPC 2.0 over `$XDG_RUNTIME_DIR/ditox/rpc.sock`. The window never opens the database.

## Install

```nix
{
  inputs.ditox.url = "github:grok-insider/ditox";

  imports = [ inputs.ditox.homeManagerModules.default ];
  programs.ditox = {
    enable = true;
    systemd.enable = true; # `ditoxd daemon`
  };
}
```

Trust `https://cache.fabrials.com/fabrials` (`fabrials:vpcHFnI0zvmDmw7Hetdbew29BbS9JObpVNkzClA8Q3k=`) to skip a local build.

Start `ditoxd daemon` with your session (the Home Manager service does) and bind a key to `ditox`.

Hyprland 0.57 and later (`hyprland.lua`):

```lua
hl.bind("SUPER + V", hl.dsp.exec_cmd("ditox"))
hl.window_rule({ match = { class = "^(dev\\.ditox)$" }, float = true, center = true })
```

Hyprland 0.56 and earlier (`hyprland.conf`):

```ini
bind = SUPER, V, exec, ditox
windowrule = float, class:^(dev\.ditox)$
```

`ditox` reads the active window before it opens. `Enter` writes the clip to the clipboard and sends the paste shortcut back to that window.

`ditoxd` needs `wl-paste` and `wl-copy` (wl-clipboard) on its `PATH`. It talks to Hyprland through its socket and falls back to `hyprctl`. The Nix package wraps them in.

## Window

| Key | Does |
|-----|------|
| Type | Search; what it found is marked in each row |
| Up, Down, Page Up, Page Down | Move the selection |
| Ctrl+Home, Ctrl+End | First and last clip |
| Enter | Paste the clip as it was copied |
| Shift+Enter | Paste its plain text |
| Ctrl+Enter | Copy it without pasting |
| Ctrl+O | Open a link |
| Tab, Shift+Tab | Next and previous filter: All, Text, Images, Pinned |
| Ctrl+1 to Ctrl+4 | Go to a filter |
| Ctrl+P | Pin or unpin |
| Delete, Ctrl+D | Delete. Delete edits the search while there is one |
| Ctrl+Z | Bring back what was just deleted |
| Ctrl+Shift+P | Pause or resume capture |
| F1 | Show every key |
| Escape | Clear the search, then close |

- The window opens on the newest clip. Pinned clips sit above it, the rest under Today, Yesterday, Last 7 days and Older.
- A row shows what its clip is: text, an image, a link, or a colour with its swatch.
- A deleted clip can come back for six seconds, or until the window closes.
- The buttons under the preview pin, delete and open a link with the pointer.
- Without a window to paste into (another compositor, `hyprctl` missing), Enter says Copy and only copies.
- The status at the bottom right says whether the daemon is watching. Click it to pause or resume.
- A missing daemon is shown as such. The window does not start one.

### Memory

The window loads only the Vulkan driver of the GPU that drives a screen, and keeps pictures at the size it draws them. On an NVIDIA desktop it opens at about 110 MB resident, of which about 80 MB is the driver's own libraries, shared with every other program that draws with the GPU. If the window cannot open with the driver it picked, it starts again with all of them. `DITOX_ALL_GPU_DRIVERS=1`, or any of the Vulkan loader's own `VK_*` driver variables, turns the choice off.

## Upgrading from 0.1

The history and the config are the same files. On its first start the 0.2 daemon:

- writes a copy of the history as it was, `ditox-v2.db.before-v5`;
- takes one line break off every text clip. The 0.1 watcher added it to everything it captured, and pasted it back.

To go back to 0.1, stop `ditoxd` and put that copy in place of `ditox-v2.db`. The terminal picker and the `[ui] terminal_command` setting are gone.

## Data

- Database: `~/.local/share/ditox/ditox-v2.db`
- Images: `~/.local/share/ditox/images-v2/`
- Config: `~/.config/ditox/config.toml` (`DITOX_CONFIG` overrides)

```toml
[history]
max_entries = 1000
delete_after_seconds = 0    # 0 keeps clips until max_entries

[paste]
enabled = true              # false only copies
buffer_ms = 120

[auto_paste]
keybind = "ctrl+v"
terminal_keybind = "ctrl+shift+v"   # sent instead when the window is a terminal
terminal_apps = ["kitty", "ghostty", "alacritty", "foot"]   # the default list is longer

[capture]
excluded_apps = ["KeePassXC", "Bitwarden"]
excluded_windows = []
secret_patterns = ["ghp_", "AKIA[A-Z0-9]{16}"]
```

Password managers, clipboard offers marked sensitive, and text that matches a secret pattern are not stored.

## Develop

```sh
nix develop
cargo test --workspace
cargo run -p ditox -- --snap /tmp/ditox.png --scene ready
cargo run -p ditox -- --list-scenes
```

Rune is pinned in `crates/ditox/Cargo.toml`. Headless snapshots (`--snap`) need Vulkan.

To try the daemon without touching your clipboard or history:

```sh
DITOX_DATA_DIR=/tmp/ditox/data DITOX_SOCKET=/tmp/ditox/rpc.sock \
DITOX_CLIPBOARD_MOCK=/tmp/ditox/clipboard cargo run -p ditoxd -- daemon
```

With `DITOX_CLIPBOARD_MOCK` the clipboard is that file, and no paste shortcut is sent.

A script can hand the running daemon a clip without touching the clipboard:

```sh
echo "from a script" | ditoxd ingest text
ditoxd ingest image < shot.png
```

## Licences

The window embeds IBM Plex Sans and IBM Plex Mono (SIL Open Font License, `crates/ditox/assets/fonts/OFL.txt`) and eight Lucide icons (ISC).
