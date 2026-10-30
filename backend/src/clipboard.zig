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
    if (!policy.pasteSendsShortcut(target_window != null)) return;
    try sendPasteShortcut(allocator, init, target_window, buffer_ms, keybind);
}

pub fn pasteBytes(allocator: std.mem.Allocator, init: std.process.Init, bytes: []const u8, mime: []const u8, target_window: ?[]const u8, buffer_ms: u32, keybind: []const u8) !void {
    try writeBytes(allocator, init, bytes, mime);
    if (init.environ_map.*.get("DITOX_CLIPBOARD_MOCK") != null) return;
    if (!policy.pasteSendsShortcut(target_window != null)) return;
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

/// How `hyprctl dispatch` wants its argument. Hyprland 0.57 evaluates it as a
/// Lua expression (`hl.dsp.*`); older releases take the dispatcher name and
/// its comma-separated arguments.
pub const DispatchSyntax = enum { legacy, lua };

const DispatchOutcome = enum {
    ok,
    failed,
    /// A legacy request that the compositor parsed as Lua and rejected.
    needs_lua,
};

const dispatch_reply_limit = 64 * 1024;

pub fn sendPasteShortcut(allocator: std.mem.Allocator, init: std.process.Init, target_window: ?[]const u8, buffer_ms: u32, keybind: []const u8) !void {
    const shortcut = try parsePasteShortcut(allocator, keybind);
    defer shortcut.deinit(allocator);

    // Asked on every paste: ditoxd outlives a Hyprland upgrade and relogin, and
    // one `hyprctl -j version` costs a few milliseconds next to `buffer_ms`.
    var syntax = detectDispatchSyntax(allocator, init);

    try std.Io.sleep(init.io, std.Io.Duration.fromMilliseconds(buffer_ms), .awake);

    if (target_window) |address| {
        // Best effort, as before: if the window is gone the shortcut still
        // goes to whatever has focus.
        const focused = focusWindow(allocator, init, syntax, address) catch .failed;
        if (focused == .needs_lua) {
            syntax = .lua;
            _ = focusWindow(allocator, init, .lua, address) catch .failed;
        }
    }

    var outcome = try sendShortcut(allocator, init, syntax, shortcut);
    if (outcome == .needs_lua) outcome = try sendShortcut(allocator, init, .lua, shortcut);
    if (outcome != .ok) return error.PasteBackFailed;
}

/// Any failure to ask (no hyprctl, no compositor, an unreadable reply) keeps
/// the legacy syntax; a Lua compositor then rejects it with a parse error and
/// the paste retries in Lua.
pub fn detectDispatchSyntax(allocator: std.mem.Allocator, init: std.process.Init) DispatchSyntax {
    const result = std.process.run(allocator, init.io, .{
        .argv = &.{ "hyprctl", "-j", "version" },
        .stdout_limit = .limited(dispatch_reply_limit),
        .stderr_limit = .limited(4096),
    }) catch return .legacy;
    defer allocator.free(result.stdout);
    defer allocator.free(result.stderr);
    switch (result.term) {
        .exited => |code| if (code != 0) return .legacy,
        else => return .legacy,
    }
    return syntaxFromVersionReply(allocator, result.stdout);
}

fn syntaxFromVersionReply(allocator: std.mem.Allocator, reply: []const u8) DispatchSyntax {
    const parsed = std.json.parseFromSlice(std.json.Value, allocator, reply, .{}) catch return .legacy;
    defer parsed.deinit();
    if (parsed.value != .object) return .legacy;
    const version = parsed.value.object.get("version") orelse return .legacy;
    if (version != .string) return .legacy;
    return syntaxForVersion(version.string);
}

fn syntaxForVersion(version: []const u8) DispatchSyntax {
    var rest = std.mem.trim(u8, version, " \t\r\n");
    if (rest.len > 0 and (rest[0] == 'v' or rest[0] == 'V')) rest = rest[1..];
    var parts = std.mem.splitScalar(u8, rest, '.');
    const major = parseLeadingNumber(parts.next() orelse return .legacy) orelse return .legacy;
    const minor = parseLeadingNumber(parts.next() orelse return .legacy) orelse return .legacy;
    if (major > 0 or minor >= 57) return .lua;
    return .legacy;
}

fn parseLeadingNumber(text: []const u8) ?u32 {
    var end: usize = 0;
    while (end < text.len and std.ascii.isDigit(text[end])) end += 1;
    if (end == 0) return null;
    return std.fmt.parseInt(u32, text[0..end], 10) catch null;
}

fn focusWindow(allocator: std.mem.Allocator, init: std.process.Init, syntax: DispatchSyntax, address: []const u8) !DispatchOutcome {
    switch (syntax) {
        .legacy => {
            const arg = try std.fmt.allocPrint(allocator, "address:{s}", .{address});
            defer allocator.free(arg);
            return runDispatch(allocator, init, .legacy, &.{ "hyprctl", "dispatch", "focuswindow", arg });
        },
        .lua => {
            const expr = try luaFocusExpr(allocator, address);
            defer allocator.free(expr);
            return runDispatch(allocator, init, .lua, &.{ "hyprctl", "dispatch", expr });
        },
    }
}

fn sendShortcut(allocator: std.mem.Allocator, init: std.process.Init, syntax: DispatchSyntax, shortcut: PasteShortcut) !DispatchOutcome {
    switch (syntax) {
        .legacy => {
            const arg = try legacyShortcutArg(allocator, shortcut);
            defer allocator.free(arg);
            return runDispatch(allocator, init, .legacy, &.{ "hyprctl", "dispatch", "sendshortcut", arg });
        },
        .lua => {
            const expr = try luaShortcutExpr(allocator, shortcut);
            defer allocator.free(expr);
            return runDispatch(allocator, init, .lua, &.{ "hyprctl", "dispatch", expr });
        },
    }
}

fn runDispatch(allocator: std.mem.Allocator, init: std.process.Init, syntax: DispatchSyntax, argv: []const []const u8) !DispatchOutcome {
    const result = try std.process.run(allocator, init.io, .{
        .argv = argv,
        .stdout_limit = .limited(dispatch_reply_limit),
        .stderr_limit = .limited(dispatch_reply_limit),
    });
    defer allocator.free(result.stdout);
    defer allocator.free(result.stderr);
    return classifyDispatch(syntax, result.term, result.stdout, result.stderr);
}

/// Legacy requests keep their old rule (exit status 0). A Lua dispatcher that
/// runs but fails still exits 0 with a `warning: ...` reply, so the Lua path
/// only counts the exact reply `ok`.
fn classifyDispatch(syntax: DispatchSyntax, term: std.process.Child.Term, stdout: []const u8, stderr: []const u8) DispatchOutcome {
    const code = switch (term) {
        .exited => |value| value,
        else => return .failed,
    };
    switch (syntax) {
        .legacy => {
            if (isLuaParseError(stdout) or isLuaParseError(stderr)) return .needs_lua;
            return if (code == 0) .ok else .failed;
        },
        .lua => {
            if (code == 0 and std.mem.eql(u8, std.mem.trim(u8, stdout, " \t\r\n"), "ok")) return .ok;
            return .failed;
        },
    }
}

/// Hyprland 0.57 wraps the request in `return hl.dispatch(...)`; a legacy
/// dispatcher string fails to parse and the error names that chunk.
fn isLuaParseError(reply: []const u8) bool {
    return std.mem.indexOf(u8, reply, "[string \"return hl.dispatch(") != null;
}

const PasteShortcut = struct {
    /// Hyprland modifier names separated by spaces, e.g. "CTRL SHIFT"; may be empty.
    mods: []const u8,
    key: []const u8,

    fn deinit(self: PasteShortcut, allocator: std.mem.Allocator) void {
        allocator.free(self.mods);
        allocator.free(self.key);
    }
};

fn parsePasteShortcut(allocator: std.mem.Allocator, keybind: []const u8) !PasteShortcut {
    const lower = try std.ascii.allocLowerString(allocator, std.mem.trim(u8, keybind, " \t\r\n"));
    defer allocator.free(lower);
    if (lower.len == 0) return error.InvalidPasteKeybind;

    var mods: std.ArrayList([]const u8) = .empty;
    defer mods.deinit(allocator);
    var main_key: ?[]const u8 = null;
    errdefer if (main_key) |key| allocator.free(key);

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
    const mods_text = try std.mem.join(allocator, " ", mods.items);
    return .{ .mods = mods_text, .key = key };
}

/// `sendshortcut` argument for Hyprland before 0.57: "MODS,KEY," (an empty
/// window target means the focused window).
fn legacyShortcutArg(allocator: std.mem.Allocator, shortcut: PasteShortcut) ![]u8 {
    return std.fmt.allocPrint(allocator, "{s},{s},", .{ shortcut.mods, shortcut.key });
}

/// Hyprland 0.57+: `hl.dsp.focus({ window = "address:0x..." })`.
fn luaFocusExpr(allocator: std.mem.Allocator, address: []const u8) ![]u8 {
    var out = std.Io.Writer.Allocating.init(allocator);
    errdefer out.deinit();
    try out.writer.writeAll("hl.dsp.focus({ window = \"address:");
    try writeLuaStringBody(&out.writer, address);
    try out.writer.writeAll("\" })");
    return out.toOwnedSlice();
}

/// Hyprland 0.57+: `hl.dsp.send_shortcut({ mods = "CTRL", key = "V" })`. No
/// `window` field, like the legacy empty target: the focused window gets it.
fn luaShortcutExpr(allocator: std.mem.Allocator, shortcut: PasteShortcut) ![]u8 {
    var out = std.Io.Writer.Allocating.init(allocator);
    errdefer out.deinit();
    try out.writer.writeAll("hl.dsp.send_shortcut({ mods = \"");
    try writeLuaStringBody(&out.writer, shortcut.mods);
    try out.writer.writeAll("\", key = \"");
    try writeLuaStringBody(&out.writer, shortcut.key);
    try out.writer.writeAll("\" })");
    return out.toOwnedSlice();
}

/// Contents of a double-quoted Lua string. The window address and the key come
/// from the command line and config, so nothing in them may end the string.
fn writeLuaStringBody(writer: *std.Io.Writer, bytes: []const u8) !void {
    for (bytes) |byte| switch (byte) {
        '\\' => try writer.writeAll("\\\\"),
        '"' => try writer.writeAll("\\\""),
        0...0x1f, 0x7f => try writer.print("\\{d:0>3}", .{byte}),
        else => try writer.writeByte(byte),
    };
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

test "dispatch syntax follows the Hyprland version" {
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxForVersion("0.56.2"));
    try std.testing.expectEqual(DispatchSyntax.lua, syntaxForVersion("0.57.0"));
    try std.testing.expectEqual(DispatchSyntax.lua, syntaxForVersion("v0.57.1"));
    try std.testing.expectEqual(DispatchSyntax.lua, syntaxForVersion("0.58.0-dev"));
    try std.testing.expectEqual(DispatchSyntax.lua, syntaxForVersion("1.0.0"));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxForVersion("0.41"));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxForVersion("0"));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxForVersion(""));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxForVersion("unknown"));
}

test "an unreadable version reply keeps the legacy syntax" {
    const allocator = std.testing.allocator;
    try std.testing.expectEqual(DispatchSyntax.lua, syntaxFromVersionReply(allocator, "{\"branch\":\"main\",\"version\":\"0.57.0\",\"tag\":\"v0.57.0\"}"));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxFromVersionReply(allocator, "{\"version\":\"0.56.2\"}"));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxFromVersionReply(allocator, ""));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxFromVersionReply(allocator, "Hyprland 0.57.0 built from branch main"));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxFromVersionReply(allocator, "[\"0.57.0\"]"));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxFromVersionReply(allocator, "{\"version\":57}"));
    try std.testing.expectEqual(DispatchSyntax.legacy, syntaxFromVersionReply(allocator, "{}"));
}

test "paste shortcuts render in both dispatch syntaxes" {
    const allocator = std.testing.allocator;
    const cases = [_]struct { keybind: []const u8, legacy: []const u8, lua: []const u8 }{
        .{ .keybind = "ctrl+v", .legacy = "CTRL,V,", .lua = "hl.dsp.send_shortcut({ mods = \"CTRL\", key = \"V\" })" },
        .{ .keybind = " Ctrl + Shift + v ", .legacy = "CTRL SHIFT,V,", .lua = "hl.dsp.send_shortcut({ mods = \"CTRL SHIFT\", key = \"V\" })" },
        .{ .keybind = "super+alt+space", .legacy = "SUPER ALT,SPACE,", .lua = "hl.dsp.send_shortcut({ mods = \"SUPER ALT\", key = \"SPACE\" })" },
        .{ .keybind = "insert", .legacy = ",INSERT,", .lua = "hl.dsp.send_shortcut({ mods = \"\", key = \"INSERT\" })" },
        .{ .keybind = "ctrl+\"", .legacy = "CTRL,\",", .lua = "hl.dsp.send_shortcut({ mods = \"CTRL\", key = \"\\\"\" })" },
        .{ .keybind = "ctrl+\\", .legacy = "CTRL,\\,", .lua = "hl.dsp.send_shortcut({ mods = \"CTRL\", key = \"\\\\\" })" },
    };
    for (cases) |case| {
        const shortcut = try parsePasteShortcut(allocator, case.keybind);
        defer shortcut.deinit(allocator);
        const legacy = try legacyShortcutArg(allocator, shortcut);
        defer allocator.free(legacy);
        try std.testing.expectEqualStrings(case.legacy, legacy);
        const lua = try luaShortcutExpr(allocator, shortcut);
        defer allocator.free(lua);
        try std.testing.expectEqualStrings(case.lua, lua);
    }
}

test "invalid paste keybinds are refused without leaking" {
    const allocator = std.testing.allocator;
    try std.testing.expectError(error.InvalidPasteKeybind, parsePasteShortcut(allocator, ""));
    try std.testing.expectError(error.InvalidPasteKeybind, parsePasteShortcut(allocator, "ctrl+shift"));
    try std.testing.expectError(error.InvalidPasteKeybind, parsePasteShortcut(allocator, "ctrl+v+x"));
    try std.testing.expectError(error.InvalidPasteKeybind, parsePasteShortcut(allocator, "v+"));
}

test "the focus target cannot end the Lua string" {
    const allocator = std.testing.allocator;
    const plain = try luaFocusExpr(allocator, "0x568e7a8e23f0");
    defer allocator.free(plain);
    try std.testing.expectEqualStrings("hl.dsp.focus({ window = \"address:0x568e7a8e23f0\" })", plain);

    const hostile = try luaFocusExpr(allocator, "0xabc\" }) hl.dsp.exit() --\\\n\x7f");
    defer allocator.free(hostile);
    try std.testing.expectEqualStrings("hl.dsp.focus({ window = \"address:0xabc\\\" }) hl.dsp.exit() --\\\\\\010\\127\" })", hostile);
}

test "a Lua dispatch only succeeds on the exact reply ok" {
    try std.testing.expectEqual(DispatchOutcome.ok, classifyDispatch(.lua, .{ .exited = 0 }, "ok\n", ""));
    try std.testing.expectEqual(DispatchOutcome.failed, classifyDispatch(.lua, .{ .exited = 0 }, "warning: window not found\n", ""));
    try std.testing.expectEqual(DispatchOutcome.failed, classifyDispatch(.lua, .{ .exited = 0 }, "", ""));
    try std.testing.expectEqual(DispatchOutcome.failed, classifyDispatch(.lua, .{ .exited = 7 }, "ok", ""));
    try std.testing.expectEqual(DispatchOutcome.failed, classifyDispatch(.lua, .{ .signal = .KILL }, "ok", ""));
}

test "a legacy dispatch keeps the exit status rule and spots a Lua compositor" {
    try std.testing.expectEqual(DispatchOutcome.ok, classifyDispatch(.legacy, .{ .exited = 0 }, "ok\n", ""));
    try std.testing.expectEqual(DispatchOutcome.ok, classifyDispatch(.legacy, .{ .exited = 0 }, "", ""));
    try std.testing.expectEqual(DispatchOutcome.failed, classifyDispatch(.legacy, .{ .exited = 1 }, "", ""));
    const parse_error = "error: 3 [string \"return hl.dispatch(sendshortcut CTRL,V,)\"]:1: ')' expected near 'CTRL'\n";
    try std.testing.expectEqual(DispatchOutcome.needs_lua, classifyDispatch(.legacy, .{ .exited = 7 }, parse_error, ""));
    try std.testing.expectEqual(DispatchOutcome.needs_lua, classifyDispatch(.legacy, .{ .exited = 7 }, "", parse_error));
}
