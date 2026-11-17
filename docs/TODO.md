# TODO

Not done:

- Clear the history, or one kind of clip, from the window.
- Copy several clips at once.
- The menu opens over its button, also after a right click on a clip.
- The list keeps the keyboard and remembers where it was only on Hyprland 0.57 and later, and nothing in the window forgets the place: the file is removed by hand.
- The options rewrite `config.toml` whole: comments are lost.
- The clipboard is read and written through `wl-paste` and `wl-copy`. A native Wayland data-control client would remove the two child processes per copy.
- Other compositors: the active window and the paste shortcut are Hyprland only.
- Syntax colours when a clip is seen whole.
- The picture on the screen needs a system that says where the list is: Hyprland or Windows. On another Wayland compositor the image is shown in the window, though the overlay itself would work there.
- No setting turns the picture on the screen off.

The schema still has the `groups` table and `entries.group_id` from 0.1. Nothing reads them.
