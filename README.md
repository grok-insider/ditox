# Ditox

Ditox is a clipboard manager for Linux/Wayland (first target: Hyprland). It is written in Rust.

- **`ditoxd`** owns the clipboard, the SQLite history, image files, and paste-back.
- **`ditox`** is the window, built with [Rune](https://github.com/anomalyco/rune): a list of clips with a search field under it. `ditox options` is a second window for the settings.
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

The rule is for the options and for an older Hyprland. On 0.57 and later Ditox asks Hyprland itself to open the list floating, holding the keyboard, and where it was left. `general.snap.enabled = true` makes Hyprland pull a floating window to the edges of the screen while it is dragged.

Hyprland 0.56 and earlier (`hyprland.conf`):

```ini
bind = SUPER, V, exec, ditox
windowrule = float, class:^(dev\.ditox)$
```

`ditox` reads the active window before it opens. `Enter` writes the clip to the clipboard and sends the paste shortcut back to that window.

`ditoxd` needs `wl-paste` and `wl-copy` (wl-clipboard) on its `PATH`. It talks to Hyprland through its socket and falls back to `hyprctl`. The Nix package wraps them in.

## The list

The window is the clips, a search field under them, and a menu. It follows [Ditto](https://github.com/sabrogden/Ditto): nothing else is on screen until it is asked for.

- A clip shows its first lines (two by default), an image a small picture. The first nine have a number.
- Pinned clips lead the list, with a mark at their left edge. The window opens on the newest clip below them.
- The menu (the button at the bottom right, a right click on a clip, or Ctrl+M) has every action with its key, narrows the list to text, images or pinned clips, pauses capture and opens the options.
- Tab shows the selected clip whole: all of a text in the window, or the image.
- Where Hyprland runs the [hypr-overlay](https://github.com/grok-insider/hypr-overlay) plugin, the image is shown large on the screen instead: drawn by the compositor in the middle of the screen the list is on, with no window around it, taking neither the keyboard nor the pointer. The list stays, and the picture follows the selection until Tab or Escape. When the middle is where the list is, the picture goes beside it. Without the plugin the image is shown in the window.
- A line appears above the search only while it has something to say: a deleted clip that can come back, capture paused, an error.
- On Hyprland 0.57 and later the list keeps the keyboard while it is on screen, wherever the pointer goes: moving the mouse over another window does not take it away. Escape, the key that opened it, or a click outside it puts it away.
- It also opens where it was left and as large as it was left: move it and resize it as any floating window, and the next one opens there, on the screen that has the keyboard. It is always put back wholly on the screen, clear of the bars. The place is in `~/.local/state/ditox/list.json`; remove the file to go back to the centre.
- Elsewhere only the size is kept, and the window closes when another window takes the keyboard. The options can turn that off.

| Key | Does |
|-----|------|
| Type | Search; what it found is marked in each row |
| Up, Down, Page Up, Page Down | Move the selection |
| Ctrl+Home, Ctrl+End | First and last clip |
| Enter | Paste the clip as it was copied |
| Shift+Enter | Paste its plain text |
| Ctrl+Enter | Copy it without pasting |
| Ctrl+1 to Ctrl+9 | Paste the clip with that number |
| Tab, F3 | See the clip whole, and back |
| Ctrl+M, F10 | Open the menu |
| Ctrl+O | Open a link |
| Ctrl+P | Pin or unpin |
| Delete, Ctrl+D | Delete. Delete edits the search while there is one |
| Ctrl+Z | Bring back what was just deleted |
| Ctrl+Shift+P | Pause or resume capture |
| Ctrl+, | Open the options |
| Escape | Leave the clip, then clear the search, then close |
| The key bound to `ditox` | Close the list, when it is open |

- A deleted clip can come back for six seconds, or until the window closes.
- Without a window to paste into (another compositor, `hyprctl` missing), Enter only copies and the menu says Copy.
- A missing daemon is shown as such. The window does not start one.

## The options

`ditox options`, Ctrl+, in the list, or Options in its menu. It is a separate window with five tabs:

- **General**: how many clips to keep, after how many days to delete them, whether a clip copied again is kept again, the theme, how many lines a clip gets in the list, and whether the list closes when it loses the keyboard.
- **Paste**: whether choosing a clip pastes it, the shortcut Ditox presses, the one for terminals, which windows are terminals, and the wait before the paste.
- **Privacy**: the apps, window titles and text patterns that are never saved.
- **Keys**: every key of the list.
- **About**: versions and where the settings and the history are.

Save asks the daemon to check the settings, write `config.toml` and live by them at once; the list reads the file the next time it opens. The daemon rewrites the file whole, so comments in it are lost. A `config.toml` that is a link into a read-only place (a system configuration put it there) is shown and cannot be saved.

### Memory

The window loads only the Vulkan driver of the GPU that drives a screen, and keeps pictures at the size it draws them: a row's picture is a few thousand pixels. On an NVIDIA desktop it opens at about 110 MB resident, of which about 80 MB is the driver's own libraries, shared with every other program that draws with the GPU. If the window cannot open with the driver it picked, it starts again with all of them. `DITOX_ALL_GPU_DRIVERS=1`, or any of the Vulkan loader's own `VK_*` driver variables, turns the choice off.

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

[ui]
theme = "system"            # or "dark", "light"
lines = 2                   # lines of each clip in the list, 1 to 5
close_on_blur = true        # the list closes when another window takes the keyboard
```

A string is in double quotes; `\"` and `\\` stand for a quote and a backslash inside one. An array is on one line.

Password managers, clipboard offers marked sensitive, and text that matches a secret pattern are not stored.

## Develop

```sh
nix develop
cargo test --workspace
cargo run -p ditox -- --snap /tmp/ditox.png --scene ready
cargo run -p ditox -- --list-scenes
cargo run -p ditox -- options --snap /tmp/options.png --scene general
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

The window embeds IBM Plex Sans and IBM Plex Mono (SIL Open Font License, `crates/ditox/assets/fonts/OFL.txt`) and four Lucide icons (ISC).
