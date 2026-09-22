const std = @import("std");
const policy = @import("policy.zig");

pub const ActiveWindow = struct {
    address: []const u8,
    class: []const u8,
    title: []const u8,

    pub fn deinit(self: ActiveWindow, allocator: std.mem.Allocator) void {
        allocator.free(self.address);
        allocator.free(self.class);
        allocator.free(self.title);
    }
};

pub const PasteFailure = enum {
    empty,
    type_unavailable,
    failed,
};

/// wl-paste exits 1 both when the selection is empty and when the source
/// advertised a type it will not serve. Those are not watcher failures.
pub fn classifyPasteFailure(stderr: []const u8) PasteFailure {
    if (std.mem.indexOf(u8, stderr, "Nothing is copied") != null) return .empty;
    if (std.mem.indexOf(u8, stderr, "not available as requested type") != null) return .type_unavailable;
    return .failed;
}

pub const ClipboardRead = union(enum) {
    bytes: []u8,
    empty,
    type_unavailable,
    /// Owned detail. Stderr when wl-paste printed some, otherwise the exit reason.
    failed: []u8,

    pub fn deinit(self: ClipboardRead, allocator: std.mem.Allocator) void {
        switch (self) {
            .bytes => |value| allocator.free(value),
            .failed => |value| allocator.free(value),
            .empty, .type_unavailable => {},
        }
    }
};

pub const MimeList = union(enum) {
    types: [][]const u8,
    empty,
    failed: []u8,

    pub fn deinit(self: MimeList, allocator: std.mem.Allocator) void {
        switch (self) {
            .types => |types| freeMimeTypes(allocator, types),
            .failed => |value| allocator.free(value),
            .empty => {},
        }
    }
};

pub fn listMimeTypes(allocator: std.mem.Allocator, init: std.process.Init) !MimeList {
    const result = std.process.run(allocator, init.io, .{
        .argv = &.{ "wl-paste", "--list-types" },
        .stderr_limit = .limited(4096),
        .stdout_limit = .limited(1024 * 1024),
    }) catch |err| {
        return .{ .failed = try allocator.dupe(u8, @errorName(err)) };
    };

    const classified = try classifyRun(allocator, result);
    switch (classified) {
        .empty => return .empty,
        .type_unavailable => return .empty,
        .failed => |detail| return .{ .failed = detail },
        .bytes => |stdout| {
            defer allocator.free(stdout);
            var list: std.ArrayList([]const u8) = .empty;
            errdefer {
                for (list.items) |item| allocator.free(item);
                list.deinit(allocator);
            }
            var lines = std.mem.splitScalar(u8, stdout, '\n');
            while (lines.next()) |line_raw| {
                const line = std.mem.trim(u8, line_raw, " \t\r\n");
                if (line.len == 0) continue;
                try list.append(allocator, try allocator.dupe(u8, line));
            }
            if (list.items.len == 0) {
                list.deinit(allocator);
                return .empty;
            }
            return .{ .types = try list.toOwnedSlice(allocator) };
        },
    }
}

pub fn freeMimeTypes(allocator: std.mem.Allocator, types: [][]const u8) void {
    for (types) |mime| allocator.free(mime);
    allocator.free(types);
}

pub fn preferredImageMime(types: []const []const u8) ?[]const u8 {
    const preferred = [_][]const u8{ "image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp" };
    for (preferred) |candidate| {
        for (types) |mime| {
            if (std.mem.eql(u8, mime, candidate)) return mime;
        }
    }
    for (types) |mime| {
        if (std.mem.startsWith(u8, mime, "image/")) return mime;
    }
    return null;
}

pub fn readText(allocator: std.mem.Allocator, init: std.process.Init) ![]u8 {
    const read = try readOffer(allocator, init, "text");
    switch (read) {
        .bytes => |value| return value,
        .empty, .type_unavailable => return allocator.dupe(u8, ""),
        .failed => |detail| {
            allocator.free(detail);
            return error.ClipboardReadFailed;
        },
    }
}

/// `mime` null reads the source's preferred offer (`wl-paste --no-newline`)
/// so a typed read that was refused can still be sniffed. `"text"` keeps the
/// text shorthand. Any other value is requested with `--type`.
pub fn readOffer(allocator: std.mem.Allocator, init: std.process.Init, mime: ?[]const u8) !ClipboardRead {
    if (mime != null and std.mem.eql(u8, mime.?, "text")) {
        if (init.environ_map.*.get("DITOX_CLIPBOARD_MOCK")) |path| {
            const bytes = try readMockClipboard(allocator, init, path);
            if (bytes.len == 0) {
                allocator.free(bytes);
                return .empty;
            }
            return .{ .bytes = bytes };
        }
    }

    var argv_buffer: [4][]const u8 = undefined;
    const argv = pasteArgv(mime, &argv_buffer);
    const result = std.process.run(allocator, init.io, .{
        .argv = argv,
        .stderr_limit = .limited(4096),
        .stdout_limit = .limited(if (mime != null and std.mem.eql(u8, mime.?, "text")) 16 * 1024 * 1024 else policy.max_image_read_bytes),
    }) catch |err| {
        return .{ .failed = try allocator.dupe(u8, @errorName(err)) };
    };
    return classifyRun(allocator, result);
}

fn pasteArgv(mime: ?[]const u8, buffer: *[4][]const u8) []const []const u8 {
    if (mime) |value| {
        if (std.mem.eql(u8, value, "text")) {
            buffer.* = .{ "wl-paste", "--no-newline", "--type", "text" };
            return buffer[0..4];
        }
        buffer.* = .{ "wl-paste", "--type", value, "" };
        return buffer[0..3];
    }
    buffer.* = .{ "wl-paste", "--no-newline", "", "" };
    return buffer[0..2];
}

fn classifyRun(allocator: std.mem.Allocator, result: std.process.RunResult) !ClipboardRead {
    defer allocator.free(result.stderr);
    switch (result.term) {
        .exited => |code| if (code == 0) {
            if (result.stdout.len == 0) {
                allocator.free(result.stdout);
                return .empty;
            }
            return .{ .bytes = result.stdout };
        },
        else => {},
    }

    defer allocator.free(result.stdout);
    switch (classifyPasteFailure(result.stderr)) {
        .empty => return .empty,
        .type_unavailable => return .type_unavailable,
        .failed => return .{ .failed = try failureDetail(allocator, result.term, result.stderr) },
    }
}

fn failureDetail(allocator: std.mem.Allocator, term: std.process.Child.Term, stderr: []const u8) ![]u8 {
    const trimmed = std.mem.trim(u8, stderr, " \t\r\n");
    const line = if (std.mem.indexOfAny(u8, trimmed, "\r\n")) |index| trimmed[0..index] else trimmed;
    if (line.len > 0) return allocator.dupe(u8, line);
    return switch (term) {
        .exited => |code| std.fmt.allocPrint(allocator, "exit {d}", .{code}),
        else => allocator.dupe(u8, "wl-paste did not exit"),
    };
}

pub fn writeText(allocator: std.mem.Allocator, init: std.process.Init, text: []const u8) !void {
    if (init.environ_map.*.get("DITOX_CLIPBOARD_MOCK")) |path| {
        try writeMockClipboard(init, path, text);
        return;
    }

    try writeBytesToClipboard(allocator, init, text, null);
}

pub fn writeBytes(allocator: std.mem.Allocator, init: std.process.Init, bytes: []const u8, mime: []const u8) !void {
    if (init.environ_map.*.get("DITOX_CLIPBOARD_MOCK")) |path| {
        try writeMockClipboard(init, path, bytes);
        const mime_path = try std.fmt.allocPrint(allocator, "{s}.mime", .{path});
        defer allocator.free(mime_path);
        try writeMockClipboard(init, mime_path, mime);
        return;
    }

    try writeBytesToClipboard(allocator, init, bytes, mime);
}

pub fn pasteText(allocator: std.mem.Allocator, init: std.process.Init, text: []const u8, target_window: ?[]const u8, buffer_ms: u32, keybind: []const u8) !void {
    try writeText(allocator, init, text);
    if (init.environ_map.*.get("DITOX_CLIPBOARD_MOCK") != null) return;
    try sendPasteShortcut(allocator, init, target_window, buffer_ms, keybind);
}

pub fn pasteBytes(allocator: std.mem.Allocator, init: std.process.Init, bytes: []const u8, mime: []const u8, target_window: ?[]const u8, buffer_ms: u32, keybind: []const u8) !void {
    try writeBytes(allocator, init, bytes, mime);
    if (init.environ_map.*.get("DITOX_CLIPBOARD_MOCK") != null) return;
    try sendPasteShortcut(allocator, init, target_window, buffer_ms, keybind);
}

fn writeBytesToClipboard(allocator: std.mem.Allocator, init: std.process.Init, bytes: []const u8, mime: ?[]const u8) !void {
    var argv_buffer: [3][]const u8 = undefined;
    const argv = if (mime) |value| blk: {
        argv_buffer = .{ "wl-copy", "--type", value };
        break :blk argv_buffer[0..3];
    } else blk: {
        argv_buffer[0] = "wl-copy";
        break :blk argv_buffer[0..1];
    };
    var child = try std.process.spawn(init.io, .{
        .argv = argv,
        .stdin = .pipe,
        .stdout = .ignore,
        .stderr = .ignore,
    });
    defer child.kill(init.io);

    var stdin_buffer: [4096]u8 = undefined;
    var stdin_writer = child.stdin.?.writer(init.io, &stdin_buffer);
    try stdin_writer.interface.writeAll(bytes);
    try stdin_writer.interface.flush();
    child.stdin.?.close(init.io);
    child.stdin = null;

    const term = try child.wait(init.io);
    switch (term) {
        .exited => |code| if (code == 0) return,
        else => {},
    }
    _ = allocator;
    return error.ClipboardWriteFailed;
}

pub fn sendPasteShortcut(allocator: std.mem.Allocator, init: std.process.Init, target_window: ?[]const u8, buffer_ms: u32, keybind: []const u8) !void {
    try std.Io.sleep(init.io, std.Io.Duration.fromMilliseconds(buffer_ms), .awake);

    if (target_window) |address| {
        const focus_arg = try std.fmt.allocPrint(allocator, "address:{s}", .{address});
        defer allocator.free(focus_arg);
        _ = std.process.run(allocator, init.io, .{
            .argv = &.{ "hyprctl", "dispatch", "focuswindow", focus_arg },
            .stdout_limit = .limited(4096),
            .stderr_limit = .limited(4096),
        }) catch {};
    }

    const shortcut = try hyprlandShortcut(allocator, keybind);
    defer allocator.free(shortcut);
    const paste = try std.process.run(allocator, init.io, .{
        .argv = &.{ "hyprctl", "dispatch", "sendshortcut", shortcut },
        .stdout_limit = .limited(4096),
        .stderr_limit = .limited(4096),
    });
    defer allocator.free(paste.stdout);
    defer allocator.free(paste.stderr);

    switch (paste.term) {
        .exited => |code| if (code == 0) return,
        else => {},
    }
    return error.PasteBackFailed;
}

fn hyprlandShortcut(allocator: std.mem.Allocator, keybind: []const u8) ![]u8 {
    const lower = try std.ascii.allocLowerString(allocator, std.mem.trim(u8, keybind, " \t\r\n"));
    defer allocator.free(lower);
    if (lower.len == 0) return error.InvalidPasteKeybind;

    var mods: std.ArrayList([]const u8) = .empty;
    defer mods.deinit(allocator);
    var main_key: ?[]const u8 = null;

    var parts = std.mem.splitScalar(u8, lower, '+');
    while (parts.next()) |raw_part| {
        const part = std.mem.trim(u8, raw_part, " \t\r\n");
        if (part.len == 0) return error.InvalidPasteKeybind;
        if (pasteModifier(part)) |modifier| {
            try mods.append(allocator, modifier);
        } else {
            if (main_key != null) return error.InvalidPasteKeybind;
            main_key = try hyprlandKey(allocator, part);
        }
    }

    const key = main_key orelse return error.InvalidPasteKeybind;
    defer allocator.free(key);

    var out = std.Io.Writer.Allocating.init(allocator);
    errdefer out.deinit();
    for (mods.items, 0..) |modifier, index| {
        if (index > 0) try out.writer.writeByte(' ');
        try out.writer.writeAll(modifier);
    }
    try out.writer.print(",{s},", .{key});
    return out.toOwnedSlice();
}

fn pasteModifier(part: []const u8) ?[]const u8 {
    if (std.mem.eql(u8, part, "ctrl") or std.mem.eql(u8, part, "control")) return "CTRL";
    if (std.mem.eql(u8, part, "shift")) return "SHIFT";
    if (std.mem.eql(u8, part, "alt") or std.mem.eql(u8, part, "option")) return "ALT";
    if (std.mem.eql(u8, part, "super") or std.mem.eql(u8, part, "meta") or std.mem.eql(u8, part, "cmd") or std.mem.eql(u8, part, "command") or std.mem.eql(u8, part, "win")) return "SUPER";
    return null;
}

fn hyprlandKey(allocator: std.mem.Allocator, part: []const u8) ![]const u8 {
    if (part.len == 1) {
        const upper = try allocator.alloc(u8, 1);
        upper[0] = std.ascii.toUpper(part[0]);
        return upper;
    }
    if (std.mem.eql(u8, part, "space")) return allocator.dupe(u8, "SPACE");
    if (std.mem.eql(u8, part, "enter") or std.mem.eql(u8, part, "return")) return allocator.dupe(u8, "RETURN");
    if (std.mem.eql(u8, part, "tab")) return allocator.dupe(u8, "TAB");
    if (std.mem.eql(u8, part, "escape") or std.mem.eql(u8, part, "esc")) return allocator.dupe(u8, "ESCAPE");
    return std.ascii.allocUpperString(allocator, part);
}

pub fn activeHyprlandAddress(allocator: std.mem.Allocator, init: std.process.Init) !?[]u8 {
    const window = try activeHyprlandWindow(allocator, init) orelse return null;
    defer window.deinit(allocator);
    return try allocator.dupe(u8, window.address);
}

pub fn activeHyprlandWindow(allocator: std.mem.Allocator, init: std.process.Init) !?ActiveWindow {
    const result = try std.process.run(allocator, init.io, .{
        .argv = &.{ "hyprctl", "-j", "activewindow" },
        .stdout_limit = .limited(1024 * 1024),
        .stderr_limit = .limited(4096),
    });
    defer allocator.free(result.stderr);

    switch (result.term) {
        .exited => |code| if (code != 0) {
            allocator.free(result.stdout);
            return null;
        },
        else => {
            allocator.free(result.stdout);
            return null;
        },
    }

    const parsed = std.json.parseFromSlice(std.json.Value, allocator, result.stdout, .{}) catch {
        allocator.free(result.stdout);
        return null;
    };
    defer parsed.deinit();
    allocator.free(result.stdout);

    const obj = parsed.value.object;
    const address = obj.get("address") orelse return null;
    if (address != .string) return null;
    const class = obj.get("class");
    const title = obj.get("title");
    return .{
        .address = try allocator.dupe(u8, address.string),
        .class = try allocator.dupe(u8, if (class != null and class.? == .string) class.?.string else ""),
        .title = try allocator.dupe(u8, if (title != null and title.? == .string) title.?.string else ""),
    };
}

fn readMockClipboard(allocator: std.mem.Allocator, init: std.process.Init, path: []const u8) ![]u8 {
    return std.Io.Dir.cwd().readFileAlloc(init.io, path, allocator, .limited(16 * 1024 * 1024)) catch |err| switch (err) {
        error.FileNotFound => allocator.dupe(u8, ""),
        else => err,
    };
}

fn writeMockClipboard(init: std.process.Init, path: []const u8, text: []const u8) !void {
    if (std.fs.path.dirname(path)) |parent| {
        if (parent.len > 0) try std.Io.Dir.cwd().createDirPath(init.io, parent);
    }
    try std.Io.Dir.cwd().writeFile(init.io, .{ .sub_path = path, .data = text });
}

test "preferredImageMime chooses image types before text" {
    const types = [_][]const u8{ "text/plain", "image/jpeg", "image/png" };
    try std.testing.expectEqualStrings("image/png", preferredImageMime(&types).?);
}

test "failureDetail keeps the first stderr line and otherwise reports the exit code" {
    const detail = try failureDetail(std.testing.allocator, .{ .exited = 1 }, "Clipboard exploded\nsecond line\n");
    defer std.testing.allocator.free(detail);
    try std.testing.expectEqualStrings("Clipboard exploded", detail);

    const bare = try failureDetail(std.testing.allocator, .{ .exited = 1 }, " \n");
    defer std.testing.allocator.free(bare);
    try std.testing.expectEqualStrings("exit 1", bare);
}

test "classifyPasteFailure separates an empty clipboard from a refused type" {
    try std.testing.expectEqual(.empty, classifyPasteFailure("Nothing is copied\n"));
    try std.testing.expectEqual(.type_unavailable, classifyPasteFailure("Clipboard content is not available as requested type \"image/png\"\n"));
    try std.testing.expectEqual(.failed, classifyPasteFailure("The compositor does not seem to implement zwlr_data_control_manager_v1\n"));
    try std.testing.expectEqual(.failed, classifyPasteFailure(""));
}
