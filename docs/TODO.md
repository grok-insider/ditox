# TODO

Not done:

- Clear the history, or one kind of clip, from the window.
- Copy several clips at once.
- The clipboard is read and written through `wl-paste` and `wl-copy`. A native Wayland data-control client would remove the two child processes per copy.
- Other compositors: the active window and the paste shortcut are Hyprland only.
- Syntax colours in the preview.

The schema still has the `groups` table and `entries.group_id` from 0.1. Nothing reads them.
