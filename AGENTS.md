# AGENTS.md

Guidance for AI coding agents working in this repository.

> This file is the source of truth for the architecture. Keep it in sync with
> the code. `docs/ROADMAP.md` is status, not a second design.

## What Ditox is

A clipboard manager for Linux/Wayland, in Rust. One Cargo workspace, two binaries:

- **`ditoxd`** (`crates/ditoxd`): clipboard, SQLite, image files, watcher, config, paste-back, JSON-RPC.
- **`ditox`** (`crates/ditox`, Rune): the only interface. Two windows, one at a time: the clip list, and `ditox options`. Neither opens SQLite.
- **`ditox-rpc`** (`crates/ditox-rpc`): the wire types, the framing and the client. Both binaries use it, so a change to a type breaks the build on both sides.

```text
ditox          (the list)     --JSON-RPC-->  ditoxd (daemon)
ditox options  (the settings) --JSON-RPC-->    storage · clipboard · watcher · config
                                               SQLite + image files + config.toml
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
cargo run -p ditox -- options --snap /tmp/options.png --scene general
cargo run -p ditox -- options --list-scenes
```

Scenes are fixtures in `crates/ditox/src/fixture.rs`, one set for each window. They do not talk to a daemon. Look at the picture after a visible change, in `ready`, `light` and `menu` at least.

The window tests (`list/tests.rs`, `options/tests.rs`) run the real views in Rune's headless `Runtime`, with no GPU. In the list, feed events reach them through `step`, the window's own turn, and `feed::take_sent()` returns what the window asked the daemon for. `feed/tests.rs` runs the feed against a real daemon in the same process, with a file for a clipboard; the options' test reads and saves settings through the same daemon.

## Layout

```text
crates/ditox-rpc/   types.rs, settings.rs, frame.rs, client.rs, hypr.rs (Hyprland's request socket)
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
| `config.rs` | TOML and JSON config: read, checked (`apply_settings`) and written back (`render`, `save`) |
| `fuzzy.rs` | The `ditox_fuzzy_score` SQLite function |
| `text.rs`, `image.rs` | Sanitising, hashing, MIME sniffing, image size |
| `process.rs` | Child processes with a timeout and an output cap |

Every request runs on one thread, in order. A `clipboard.wake` asks for a capture; the capture runs 40 ms after the last wake and at most 250 ms after the first. The search calls `ditox_fuzzy_score` several times per row with the same arguments; the function remembers its last two answers.

### Windows

| Module | Purpose |
|--------|---------|
| `main.rs` | Which window: `ditox` is the list, `ditox options` the settings. `AppSpec` of each |
| `list.rs` | The list's state, key map, menu entries, `apply` (what a feed event changes) |
| `list/view.rs` | The list's view: rows, the clip seen whole, the line above the search, the search bar, the menu |
| `options.rs` | The options' state, what its fields mean as `Settings`, and its thread to the daemon |
| `options/view.rs` | The options' view: tabs, fields, boxes, choices, Save and Cancel |
| `theme.rs` | Fabrials tokens: palette, type scale, radii, icons, embedded IBM Plex |
| `model.rs` | Pure helpers: what a row says, the caption of a clip seen whole, links, search marks, selection |
| `feed.rs` | The list's background thread that talks to the daemon |
| `picture.rs` | The threads that decode pictures: one for the clip seen whole, one for the rows |
| `startup.rs` | Before the window: which Vulkan drivers load, how freed memory is returned |
| `fixture.rs` | Snapshot scenes |

The list polls `history.pulse` every 250 ms and asks for the list again only when the revision moved. Lists are compact (400 characters of text): a row needs no more. While a clip is seen whole (`peek`), its full text or image comes from `entries.get` and `entries.get_image`: the window says which clip it waits for (`Want`); the feed fetches only the last one asked, retries a failure on the next pulse, and reports a clip that is gone or unreadable.

A row of an image clip shows a small picture. The list tells the window which rows are in view (`on_visible_rows`); the window asks for the pictures those rows and a few past them lack (`Thumbs`, a list that replaces the one before). The feed fetches one image a turn, hands it to a second decode thread, and lets at most two wait there: each is a whole image file in memory. A picture that cannot be made is said once (`NoThumb`) and not asked for again.

A delete is the feed's to carry out. The window takes the row off and sends `Delete`; the feed keeps the id for `UNDO_TIME`, leaves it out of every list that arrives, and deletes it when the time is up or the window closes (`feed::finish`, from `on_close`). `Restore` takes it back.

The menu is drawn in the window (Rune has no native pop-up menu on Linux). Its entries are data (`State::menu_items`): the view draws them, the keyboard walks them and `choose` runs one. While it is open the keyboard is the menu's.

The options window loads the settings once (`config.get`), keeps what the user types, and on Save sends all of them (`config.set`). The daemon checks them, writes the file, and only then lives by them; a refusal comes back as a sentence under the fields. The list never asks the daemon for settings: it reads the `[ui]` section of the file before its first frame (`ditox_rpc::Ui::load`), so the theme is right from the first pixel.

### Memory

Measure before and after a change here; the numbers are in `docs/ROADMAP.md`.

- `startup::prepare` runs first in `main`. It sets `VK_LOADER_DRIVERS_SELECT` to the driver of the cards that drive a screen, read from `/sys/class/drm`. Left alone the Vulkan loader opens every installed driver, and Mesa's bring LLVM with them. A card it does not know means no choice. When the window cannot open with the choice, the watchdog starts the program again with every driver.
- The allocator is told to use two arenas and fixed thresholds, so a decoded picture is handed back when freed.
- A picture seen whole is decoded on its own thread and kept at most 1280 pixels on its long side (2048 at a scale above 1.25): `picture::fit`. The window keeps the last 4, up to 24 MiB. A picture above 256 MiB decoded is described, not drawn.
- A row's picture is made at the size it is drawn, at most 240 by 60 points (`picture::fit_within`). The window keeps 96 of them, up to 12 MiB.
- `Client::image` decodes the base64 where it lies in the response, and the frame reader hands over its buffer instead of copying it.
- The window asks Hyprland for the active window through its socket (`ditox_rpc::hypr`), not `hyprctl`: a process costs tens of milliseconds before the window opens.

## Design

The list is laid out after Ditto, the Windows clipboard manager: clips from edge to edge with every other row a shade apart, a number in a gutter for the first nine, the search at the bottom, and everything else in one menu. The rule is that nothing is on screen until it has something to say: no preview pane, no filters, no key hints, no status. The settings are a window of their own.

Both windows take their looks from Fabrials UI 0.11 (`libs/fabrials-ui/DESIGN.md` in the house workspace): deep Stormwall neutrals, the selected row as a tint, a mono label voice for numbers and metadata, 8 px buttons, 6 px fields and tags, a 10 px menu, IBM Plex. Take colours, sizes and radii from `theme.rs`; do not write colour or type literals in a view. Text on screen is English.

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
- Ctrl and a digit pressed while a search is on its way pastes that number of the results when they arrive, if there are that many. A second press on a row is not kept for a list that has not arrived.
- The list closes when another window takes the keyboard (`[ui] close_on_blur`), and only if it had the keyboard before. A window left open and unfocused would take a second Super+V for a new one and leave typing to go to whatever is under it.
- Asked for while the options window has the keyboard, the list opens with nowhere to paste: the options are this program too.
- The daemon is the only writer of `config.toml`. It takes settings whole or not at all, writes the file beside itself and renames it, and refuses a file that is a link. What `render` writes, `apply_file` reads back to the same config: a test holds the two together.
- A string in the config is read with its quotes: a comma, a `#` or a bracket inside one is text.
- A window that cannot open exits. Rune would keep the process alive without one.

## RPC

The method table is `crates/ditoxd/src/rpc.rs`: `health.check`, `config.get`, `config.set`, `history.pulse`, `history.pause`, `history.resume`, `entries.list`, `entries.get`, `entries.get_image`, `entries.paste`, `entries.favorite`, `entries.sticky`, `entries.delete`, `clipboard.wake` and `clipboard.ingest`. Each has a caller: the list, the options, the watcher (`ditoxd ingest wake`) or `ditoxd ingest text|image`. A method nothing calls is removed, not kept for later. Result types live in `crates/ditox-rpc/src/types.rs` and `settings.rs`, and the calls the windows make in `client.rs`. A method that changes the history must not be listed in `changes_history`'s read-only set, or the window will not refresh.

## Data

- `~/.local/share/ditox/ditox-v2.db`
- `~/.local/share/ditox/images-v2/`
- `~/.config/ditox/config.toml`

Tests use temporary directories. To run the daemon by hand, set `DITOX_DATA_DIR`, `DITOX_SOCKET` and `DITOX_CLIPBOARD_MOCK` (see the README). Do not point a development daemon at the real database.

Do not open the window on the user's desktop to test it: it reads the active window and `Enter` pastes into it. Use `--snap`, the headless tests, or a compositor of your own with a mock daemon and `DITOX_CONFIG` pointed at a file of your own: Save in the options writes it.

In a headless compositor with no keyboard, a tool that types through a virtual keyboard (`wtype`) takes the seat's only keyboard away when it exits, and the list closes as it should when the keyboard goes. Keep one virtual keyboard alive for the length of the test.
