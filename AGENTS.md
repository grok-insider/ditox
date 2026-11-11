# AGENTS.md

Guidance for AI coding agents working in this repository.

> This file is the source of truth for the architecture. Keep it in sync with
> the code. `docs/ROADMAP.md` is status, not a second design.

## What Ditox is

A clipboard manager for Linux/Wayland, in Rust. One Cargo workspace, two binaries:

- **`ditoxd`** (`crates/ditoxd`): clipboard, SQLite, image files, watcher, config, paste-back, JSON-RPC.
- **`ditox`** (`crates/ditox`, Rune): the only interface. It is a window. It does not open SQLite.
- **`ditox-rpc`** (`crates/ditox-rpc`): the wire types, the framing and the client. Both binaries use it, so a change to a type breaks the build on both sides.

```text
ditox (Rune window)  --JSON-RPC-->  ditoxd (daemon)
                                      storage · clipboard · watcher
                                      SQLite + image files
```

The socket is `$XDG_RUNTIME_DIR/ditox/rpc.sock` (Content-Length framed JSON-RPC 2.0, mode 0600).

## Build and test

```bash
nix develop
cargo test --workspace
cargo clippy --workspace --all-targets
```

`nix build` builds the workspace, runs its tests, and ships `bin/ditox` and `bin/ditoxd` together. Home Manager starts `ditoxd daemon`.

Headless UI check (needs Vulkan):

```bash
cargo run -p ditox -- --snap /tmp/ditox.png --scene ready
cargo run -p ditox -- --list-scenes
```

Scenes are fixtures in `crates/ditox/src/fixture.rs`. They do not talk to a daemon. Look at the picture after a visible change, in `ready` and `light` at least.

The window tests (`crates/ditox/src/tests.rs`) run the real view in Rune's headless `Runtime`, with no GPU. Feed events reach them through `step`, the window's own turn, and `feed::take_sent()` returns what the window asked the daemon for. `crates/ditox/src/feed/tests.rs` runs the feed against a real daemon in the same process, with a file for a clipboard.

## Layout

```text
crates/ditox-rpc/   types.rs, frame.rs, client.rs, hypr.rs (Hyprland's request socket)
crates/ditoxd/      the daemon
crates/ditox/       the window
docs/               ROADMAP, TODO
flake.nix           package, devShell, homeManagerModules
```

### Daemon

| Module | Purpose |
|--------|---------|
| `main.rs` | `daemon`, `ingest wake`, `ingest text`, `ingest image` |
| `server.rs` | Socket, one thread per client, the single loop that owns the store, the `wl-paste --watch` child |
| `rpc.rs` | Method table and parameter checks |
| `app.rs` | Capture, paste, copy; the `Desktop` trait |
| `storage.rs` | SQLite, FTS, retention, prune queue, migrations |
| `blob.rs` | Content-addressed image files |
| `clipboard.rs` | `wl-paste` and `wl-copy`, or a file with `DITOX_CLIPBOARD_MOCK` |
| `hyprland.rs` | Active window, the class of a window, and the paste shortcut: 0.57 Lua dispatch and the earlier syntax, over Hyprland's socket or `hyprctl` |
| `policy.rs` | What may be stored or pasted. Pure. |
| `config.rs` | TOML and JSON config |
| `fuzzy.rs` | The `ditox_fuzzy_score` SQLite function |
| `text.rs`, `image.rs` | Sanitising, hashing, MIME sniffing, image size |
| `process.rs` | Child processes with a timeout and an output cap |

Every request runs on one thread, in order. A `clipboard.wake` asks for a capture; the capture runs 40 ms after the last wake and at most 250 ms after the first. The search calls `ditox_fuzzy_score` several times per row with the same arguments; the function remembers its last two answers.

### Window

| Module | Purpose |
|--------|---------|
| `main.rs` | State, key map, `apply` (what a feed event changes), `AppSpec` |
| `ui.rs` | The view |
| `theme.rs` | Fabrials tokens: palette, type scale, radii, icons, embedded IBM Plex |
| `model.rs` | Pure helpers: titles, ages, list rows and their headings, what a clip is, search marks, selection |
| `feed.rs` | The background thread that talks to the daemon |
| `picture.rs` | The thread that decodes a picture and scales it to the preview |
| `startup.rs` | Before the window: which Vulkan drivers load, how freed memory is returned |
| `fixture.rs` | Snapshot scenes |

The window polls `history.pulse` every 250 ms and asks for the list again only when the revision moved. Lists are compact (400 characters of text); the selected clip's full text and image come from `entries.get` and `entries.get_image`. The window says which clip it waits for (`Want`); the feed fetches only the last one asked, retries a failure on the next pulse, and reports a clip that is gone or unreadable.

A delete is the feed's to carry out. The window takes the row off and sends `Delete`; the feed keeps the id for `UNDO_TIME`, leaves it out of every list that arrives, and deletes it when the time is up or the window closes (`feed::finish`, from `on_close`). `Restore` takes it back.

### Memory

Measure before and after a change here; the numbers are in `docs/ROADMAP.md`.

- `startup::prepare` runs first in `main`. It sets `VK_LOADER_DRIVERS_SELECT` to the driver of the cards that drive a screen, read from `/sys/class/drm`. Left alone the Vulkan loader opens every installed driver, and Mesa's bring LLVM with them. A card it does not know means no choice. When the window cannot open with the choice, the watchdog starts the program again with every driver.
- The allocator is told to use two arenas and fixed thresholds, so a decoded picture is handed back when freed.
- A picture is decoded on its own thread and kept at most 1280 pixels on its long side (2048 at a scale above 1.25): `picture::fit`. The window keeps the last 4, up to 24 MiB. A picture above 256 MiB decoded is described, not drawn.
- `Client::image` decodes the base64 where it lies in the response, and the frame reader hands over its buffer instead of copying it.
- The window asks Hyprland for the active window through its socket (`ditox_rpc::hypr`), not `hyprctl`: a process costs tens of milliseconds before the window opens.

## Design

The window follows Fabrials UI 0.11 (`libs/fabrials-ui/DESIGN.md` in the house workspace): deep Stormwall neutrals, one 12 px plate for the preview, 8 px controls, 6 px tags, the selected row as a tint, a mono label voice for metadata, IBM Plex. Take colours, sizes and radii from `theme.rs`; do not write literals in `ui.rs`. Text on screen is English.

## Rules that still hold

- The window never reads the database or image files. Image bytes come from `entries.get_image`.
- A delete can be undone for six seconds and happens when the window closes at the latest. A list that arrives meanwhile must not bring the row back.
- A terminal is pasted into with `terminal_paste_keybind` (Ctrl+Shift+V). The daemon looks up the class of the target window; a name in `terminal_apps` matches the whole class or the part after its last dot.
- The daemon asks Hyprland through its socket and falls back to `hyprctl` only when there is no socket. A request the socket did not answer in time is not asked again another way: it could act twice.
- The watcher prefers an image over the text alternative. An announced image that cannot be read is not replaced by its URL.
- MIME is sniffed. A lying announcement is not stored as an image.
- Entries are SHA-256 deduplicated.
- Image files are content-addressed, written atomically, and pruned only when no row references them.
- A capture error must not kill the watcher. It is logged and shown as `watcher.status.last_error`.
- The clip is on the clipboard before the paste shortcut is sent. If the shortcut fails, the clip stays there and the window says so.
- With `DITOX_CLIPBOARD_MOCK` the daemon touches neither the clipboard nor Hyprland.
- The schema version is 5. Schema 4 is what 0.1 wrote; its watcher added a line break to every text it captured, and the step to 5 takes that one break off. Before any step from an older schema the daemon writes `ditox-v2.db.before-v5` next to the database, because an older daemon refuses a newer schema. A migration has a test that opens an older database.
- Text is read with `wl-paste --no-newline`. Without it every clip grows a line break and pastes it back.
- Every child process and every request to Hyprland has a deadline, on the thread that owns the store: `wl-paste` 5 s, Hyprland 3 s, `wl-copy` 10 s.
- An answer to an older search is dropped (`generation`), so fast typing cannot leave the selection on the wrong clip.
- Enter pressed while a search is on its way pastes the first result when it arrives, for 1.5 s. Without a daemon, or after the selection moves, Enter is not kept.
- A press on plain content must not take the keyboard from the search field: the root keeps focus on press.
- A window that cannot open exits. Rune would keep the process alive without one.

## RPC

The method table is `crates/ditoxd/src/rpc.rs`: `health.check`, `history.pulse`, `history.pause`, `history.resume`, `entries.list`, `entries.get`, `entries.get_image`, `entries.paste`, `entries.favorite`, `entries.sticky`, `entries.delete`, `clipboard.wake` and `clipboard.ingest`. Each has a caller: the window, the watcher (`ditoxd ingest wake`) or `ditoxd ingest text|image`. A method nothing calls is removed, not kept for later. Result types live in `crates/ditox-rpc/src/types.rs` and the calls the window makes in `client.rs`. A method that changes the history must not be listed in `changes_history`'s read-only set, or the window will not refresh.

## Data

- `~/.local/share/ditox/ditox-v2.db`
- `~/.local/share/ditox/images-v2/`
- `~/.config/ditox/config.toml`

Tests use temporary directories. To run the daemon by hand, set `DITOX_DATA_DIR`, `DITOX_SOCKET` and `DITOX_CLIPBOARD_MOCK` (see the README). Do not point a development daemon at the real database.

Do not open the window on the user's desktop to test it: it reads the active window and `Enter` pastes into it. Use `--snap`, the headless tests, or a nested compositor with a mock daemon.
