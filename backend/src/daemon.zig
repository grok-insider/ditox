const std = @import("std");
const core = @import("ditox_core");

const Io = std.Io;

pub fn main(init: std.process.Init) !void {
    const allocator = init.gpa;
    const args = try init.minimal.args.toSlice(init.arena.allocator());

    if (args.len > 1 and std.mem.eql(u8, args[1], "serve")) {
        try serveStdio(allocator, init);
        return;
    }
    if (args.len > 1 and std.mem.eql(u8, args[1], "watch")) {
        try watchClipboard(allocator, init);
        return;
    }
    if (args.len > 1 and std.mem.eql(u8, args[1], "ingest")) {
        const kind = if (args.len > 2) args[2] else "text";
        if (std.mem.eql(u8, kind, "wake")) {
            try ingestWake(allocator, init);
        } else {
            try ingestStdin(allocator, init, kind);
        }
        return;
    }
    if (args.len > 1 and (std.mem.eql(u8, args[1], "daemon") or std.mem.eql(u8, args[1], "run"))) {
        try runPersistentDaemon(allocator, init, args[0]);
        return;
    }

    var stdout_buffer: [1024]u8 = undefined;
    var stdout_writer = Io.File.stdout().writer(init.io, &stdout_buffer);
    const stdout = &stdout_writer.interface;
    try stdout.writeAll("usage: ditoxd serve --stdio | ditoxd watch | ditoxd daemon | ditoxd ingest <text|image>\n");
    try stdout.flush();
}

/// Serves JSON-RPC over stdio. Content-Length framed requests are answered in
/// a loop over one long-lived store, so a TUI session holds a single
/// connection instead of spawning a process per call. For compatibility,
/// leftover unframed input at EOF is answered once (the old one-shot mode).
fn serveStdio(allocator: std.mem.Allocator, init: std.process.Init) !void {
    var stdout_buffer: [4096]u8 = undefined;
    var stdout_writer = Io.File.stdout().writer(init.io, &stdout_buffer);
    const stdout = &stdout_writer.interface;

    var opened = try core.app.openStore(allocator, init);
    defer opened.cfg.deinit();
    defer opened.store.close();

    var pending: std.ArrayList(u8) = .empty;
    defer pending.deinit(allocator);
    var buf: [4096]u8 = undefined;

    while (true) {
        while (framedRequestSpan(pending.items)) |span| {
            try respondStdio(allocator, init, stdout, &opened, pending.items[span.body_start..][0..span.body_len]);
            try pending.replaceRange(allocator, 0, span.body_start + span.body_len, &.{});
        }
        const n = std.posix.read(std.posix.STDIN_FILENO, &buf) catch break;
        if (n == 0) break;
        try pending.appendSlice(allocator, buf[0..n]);
    }

    const leftover = std.mem.trim(u8, pending.items, " \t\r\n");
    if (leftover.len > 0) try respondStdio(allocator, init, stdout, &opened, leftover);
}

fn respondStdio(allocator: std.mem.Allocator, init: std.process.Init, stdout: *Io.Writer, opened: anytype, request: []const u8) !void {
    const response = try core.rpc.handleOpened(allocator, init, request, opened);
    defer allocator.free(response);
    const framed = try core.rpc.frame(allocator, response);
    defer allocator.free(framed);
    try stdout.writeAll(framed);
    try stdout.flush();
}

const FrameSpan = struct { body_start: usize, body_len: usize };

fn framedRequestSpan(bytes: []const u8) ?FrameSpan {
    var sep_len: usize = 4;
    const header_end = std.mem.indexOf(u8, bytes, "\r\n\r\n") orelse blk: {
        sep_len = 2;
        break :blk std.mem.indexOf(u8, bytes, "\n\n") orelse return null;
    };
    const length = parseContentLength(bytes[0..header_end]) orelse return null;
    const body_start = header_end + sep_len;
    if (bytes.len < body_start + length) return null;
    return .{ .body_start = body_start, .body_len = length };
}

fn parseContentLength(header: []const u8) ?usize {
    var lines = std.mem.splitScalar(u8, header, '\n');
    while (lines.next()) |raw_line| {
        const line = std.mem.trim(u8, raw_line, " \t\r");
        const colon = std.mem.indexOfScalar(u8, line, ':') orelse continue;
        const name = std.mem.trim(u8, line[0..colon], " \t");
        if (!std.ascii.eqlIgnoreCase(name, "content-length")) continue;
        return std.fmt.parseInt(usize, std.mem.trim(u8, line[colon + 1 ..], " \t"), 10) catch null;
    }
    return null;
}

fn watchClipboard(allocator: std.mem.Allocator, init: std.process.Init) !void {
    var opened = try core.app.openStore(allocator, init);
    defer opened.cfg.deinit();
    defer opened.store.close();

    try opened.store.markWatcherPid(@as(i64, @intCast(std.os.linux.getpid())));
    defer opened.store.clearWatcherPid() catch {};

    // Delegate to the shared loop (keeps logic single-source; legacy `watch` still works for compat).
    try runCaptureLoop(allocator, init, opened.cfg, &opened.store);
}

const OfferClass = enum { bytes, empty, unavailable, failed };

/// What a capture tick does with one clipboard read.
const CaptureDecision = enum {
    /// Bytes are a usable image, or text worth storing.
    store,
    /// Typed image read did not serve bytes. One untyped read follows.
    try_untyped,
    /// Nothing to store and nothing to report. Do not read text.
    skip,
    /// Real wl-paste or compositor failure.
    fail,
    /// No image was announced. The text offer may be read.
    read_text,
};

fn offerClass(read: *const core.clipboard.ClipboardRead) OfferClass {
    return switch (read.*) {
        .bytes => .bytes,
        .empty => .empty,
        .type_unavailable => .unavailable,
        .failed => .failed,
    };
}

fn afterAnnouncedImage(class: OfferClass) CaptureDecision {
    return switch (class) {
        .bytes => .store,
        .empty, .unavailable => .try_untyped,
        .failed => .fail,
    };
}

/// Typed bytes are stored only when they are a real image. A sniffable
/// announcement that does not match the payload gets one untyped retry
/// instead of being saved as text.
fn afterTypedImageBytes(usable_image: bool) CaptureDecision {
    return if (usable_image) .store else .try_untyped;
}

fn afterUntypedFallback(class: OfferClass, sniffed_image: bool) CaptureDecision {
    return switch (class) {
        .bytes => if (sniffed_image) .store else .skip,
        .empty, .unavailable => .skip,
        .failed => .fail,
    };
}

fn afterTextOffer(class: OfferClass) CaptureDecision {
    return switch (class) {
        .bytes => .store,
        .empty, .unavailable => .skip,
        .failed => .fail,
    };
}

const CaptureStep = union(enum) {
    done,
    read_text,
    failed: []u8,
};

fn captureImage(
    allocator: std.mem.Allocator,
    init: std.process.Init,
    cfg: core.config.Config,
    store: *core.storage.Storage,
    last_hash: *?[64]u8,
) !CaptureStep {
    const listed = try core.clipboard.listMimeTypes(allocator, init);
    switch (listed) {
        .empty => return .done,
        .failed => |reason| return .{ .failed = reason },
        .types => |types| {
            defer core.clipboard.freeMimeTypes(allocator, types);
            const announced = core.clipboard.preferredImageMime(types) orelse return .read_text;
            const typed = try core.clipboard.readOffer(allocator, init, announced);
            switch (afterAnnouncedImage(offerClass(&typed))) {
                .fail => {
                    const reason = typed.failed;
                    return .{ .failed = reason };
                },
                .store => {
                    const bytes = typed.bytes;
                    defer allocator.free(bytes);
                    const usable = bytes.len > 0 and (core.util.detectImageMime(bytes) != null or !core.util.isSniffableImageMime(announced));
                    if (afterTypedImageBytes(usable) == .store) {
                        if (try storeImageOffer(allocator, cfg, store, last_hash, announced, bytes)) return .done;
                    }
                },
                .try_untyped => typed.deinit(allocator),
                .skip, .read_text => typed.deinit(allocator),
            }

            const untyped = try core.clipboard.readOffer(allocator, init, null);
            const sniffed = switch (untyped) {
                .bytes => |bytes| core.util.detectImageMime(bytes) != null,
                else => false,
            };
            switch (afterUntypedFallback(offerClass(&untyped), sniffed)) {
                .store => {
                    const bytes = untyped.bytes;
                    defer allocator.free(bytes);
                    const mime = core.util.detectImageMime(bytes).?;
                    _ = try storeResolvedImage(allocator, cfg, store, last_hash, mime, bytes);
                    return .done;
                },
                .fail => return .{ .failed = untyped.failed },
                .skip, .try_untyped, .read_text => {
                    untyped.deinit(allocator);
                    return .done;
                },
            }
        },
    }
}

fn storeImageOffer(
    allocator: std.mem.Allocator,
    cfg: core.config.Config,
    store: *core.storage.Storage,
    last_hash: *?[64]u8,
    announced: []const u8,
    bytes: []const u8,
) !bool {
    if (bytes.len == 0) return false;
    // Trust magic bytes over the announced type. A sniffable announcement
    // that does not match is not stored. Formats without a signature, such
    // as image/svg+xml, keep the announced type.
    const sniffed = core.util.detectImageMime(bytes);
    const mime = sniffed orelse mime: {
        if (core.util.isSniffableImageMime(announced)) return false;
        break :mime announced;
    };
    return try storeResolvedImage(allocator, cfg, store, last_hash, mime, bytes);
}

fn storeResolvedImage(
    allocator: std.mem.Allocator,
    cfg: core.config.Config,
    store: *core.storage.Storage,
    last_hash: *?[64]u8,
    mime: []const u8,
    bytes: []const u8,
) !bool {
    const hash = core.util.sha256Hex(bytes);
    if (last_hash.* != null and std.mem.eql(u8, &last_hash.*.?, &hash)) return true;
    if (try shouldSkipSelfWrite(allocator, store, &hash)) {
        last_hash.* = hash;
        return true;
    }
    _ = try store.addImage(cfg, mime, bytes, core.util.imageMetadata(bytes, mime));
    last_hash.* = hash;
    return true;
}

fn captureText(
    allocator: std.mem.Allocator,
    init: std.process.Init,
    cfg: core.config.Config,
    store: *core.storage.Storage,
    last_hash: *?[64]u8,
) !CaptureStep {
    const read = try core.clipboard.readOffer(allocator, init, "text");
    switch (afterTextOffer(offerClass(&read))) {
        .store => {
            const text = read.bytes;
            defer allocator.free(text);
            if (text.len == 0) return .done;
            const hash = core.util.sha256Hex(text);
            if (last_hash.* != null and std.mem.eql(u8, &last_hash.*.?, &hash)) return .done;
            if (try shouldSkipSelfWrite(allocator, store, &hash)) {
                last_hash.* = hash;
                return .done;
            }
            _ = try store.addText(cfg, text);
            last_hash.* = hash;
            return .done;
        },
        .fail => return .{ .failed = read.failed },
        .skip, .try_untyped, .read_text => {
            read.deinit(allocator);
            return .done;
        },
    }
}

const SelfWriteDecision = enum { store, skip };

fn decideSelfWrite(stored: ?[]const u8, seen: []const u8) SelfWriteDecision {
    const previous = stored orelse return .store;
    if (std.mem.eql(u8, previous, seen)) return .skip;
    return .store;
}

fn shouldSkipSelfWrite(allocator: std.mem.Allocator, store: *core.storage.Storage, hash: []const u8) !bool {
    const self_hash = try store.selfWriteHash() orelse return false;
    defer allocator.free(self_hash);
    // A different clipboard value retires the saved hash. Leaving it set made
    // a later intentional copy of that same text look like our own paste.
    try store.clearSelfWriteHash();
    return decideSelfWrite(self_hash, hash) == .skip;
}

fn isActiveWindowExcluded(allocator: std.mem.Allocator, init: std.process.Init, cfg: core.config.Config) !bool {
    const window = try core.clipboard.activeHyprlandWindow(allocator, init) orelse return false;
    defer window.deinit(allocator);
    return matchesAny(window.class, cfg.excluded_apps) or matchesAny(window.title, cfg.excluded_windows);
}

fn matchesAny(value: []const u8, patterns: []const []const u8) bool {
    for (patterns) |pattern| {
        if (containsIgnoreCase(value, pattern)) return true;
    }
    return false;
}

fn containsIgnoreCase(value: []const u8, pattern: []const u8) bool {
    if (pattern.len == 0 or pattern.len > value.len) return false;
    var start: usize = 0;
    while (start + pattern.len <= value.len) : (start += 1) {
        for (pattern, 0..) |needle, offset| {
            if (std.ascii.toLower(value[start + offset]) != std.ascii.toLower(needle)) break;
        } else {
            return true;
        }
    }
    return false;
}

test "the clipboard watch asks ingest to wake" {
    const argv = watchArgv("ditoxd");
    try std.testing.expectEqualStrings("wl-paste", argv[0]);
    try std.testing.expectEqualStrings("--watch", argv[1]);
    try std.testing.expectEqualStrings("ditoxd", argv[2]);
    try std.testing.expectEqualStrings("ingest", argv[3]);
    try std.testing.expectEqualStrings("wake", argv[4]);
}

test "framedRequestSpan parses complete Content-Length frames only" {
    // Incomplete header.
    try std.testing.expect(framedRequestSpan("Content-Length: 5\r\n") == null);
    // Header complete, body incomplete.
    try std.testing.expect(framedRequestSpan("Content-Length: 5\r\n\r\nab") == null);
    // No Content-Length header.
    try std.testing.expect(framedRequestSpan("X-Other: 1\r\n\r\n{}") == null);

    const exact = framedRequestSpan("Content-Length: 2\r\n\r\n{}") orelse return error.TestUnexpectedResult;
    try std.testing.expectEqual(@as(usize, 21), exact.body_start);
    try std.testing.expectEqual(@as(usize, 2), exact.body_len);

    // Case-insensitive header name and a second pipelined request behind it.
    const pipelined = framedRequestSpan("content-length: 4\r\n\r\nabcdContent-Length: 2\r\n\r\n{}") orelse return error.TestUnexpectedResult;
    try std.testing.expectEqual(@as(usize, 21), pipelined.body_start);
    try std.testing.expectEqual(@as(usize, 4), pipelined.body_len);

    // Bare-newline separator tolerated.
    const bare = framedRequestSpan("Content-Length: 2\n\n{}") orelse return error.TestUnexpectedResult;
    try std.testing.expectEqual(@as(usize, 19), bare.body_start);
    try std.testing.expectEqual(@as(usize, 2), bare.body_len);
}

test "a different clipboard value retires a stale self-write hash" {
    try std.testing.expectEqual(.store, decideSelfWrite(null, "abc"));
    try std.testing.expectEqual(.skip, decideSelfWrite("abc", "abc"));
    try std.testing.expectEqual(.store, decideSelfWrite("abc", "def"));
}

test "excluded app and window matching is case-insensitive" {
    const apps = [_][]const u8{ "Bitwarden", "KeePassXC" };
    const windows = [_][]const u8{"Secret Window"};

    try std.testing.expect(matchesAny("bitwarden desktop", &apps));
    try std.testing.expect(matchesAny("org.keepassxc.KeePassXC", &apps));
    try std.testing.expect(matchesAny("private SECRET window title", &windows));
    try std.testing.expect(!matchesAny("Alacritty", &apps));
    try std.testing.expect(!matchesAny("normal browser tab", &windows));
}

/// Proper persistent single-owner daemon (implements the approved architectural plan).
/// - Owns ONE long-lived Storage connection for the lifetime of the daemon (this is the core fix for the "database is locked" + SQLiteFailure storm that made the watcher go stale).
/// - Runs the FULL clipboard capture watcher loop (captureImageFirst + captureText + add* writes + exclude/pause/last_hash/self-write guards) against that single owned *Storage.
/// - `ditoxd serve --stdio` and legacy `ditoxd watch` remain for compatibility / one-offs.
/// - This eliminates the N-process writer contention at the source for the capture workload.
const Client = struct {
    fd: std.posix.fd_t,
    pending: std.ArrayList(u8),
};

fn runPersistentDaemon(allocator: std.mem.Allocator, init: std.process.Init, exe: []const u8) !void {
    var opened = try core.app.openStore(allocator, init);
    try opened.store.markWatcherPid(@as(i64, @intCast(std.os.linux.getpid())));
    defer opened.store.clearWatcherPid() catch {};
    opened.store.clearWatcherError() catch {};

    const socket_path = try rpcSocketPath(allocator, init);
    defer allocator.free(socket_path);
    var server = try listenRpc(allocator, init, socket_path);
    defer server.deinit(init.io);

    const watch_argv = watchArgv(exe);
    var watch = try spawnWatch(allocator, init, exe, &watch_argv);
    defer watch.kill(init.io);

    var clients: std.ArrayList(Client) = .empty;
    defer {
        for (clients.items) |*client| {
            _ = std.posix.system.close(client.fd);
            client.pending.deinit(allocator);
        }
        clients.deinit(allocator);
    }

    while (true) {
        const polls = try allocator.alloc(std.posix.pollfd, clients.items.len + 1);
        defer allocator.free(polls);
        polls[0] = .{ .fd = server.socket.handle, .events = std.posix.POLL.IN, .revents = 0 };
        for (clients.items, 0..) |client, index| {
            polls[index + 1] = .{ .fd = client.fd, .events = std.posix.POLL.IN, .revents = 0 };
        }
        const ready = std.posix.poll(polls, 1000) catch 0;
        opened.store.markWatcherSeen() catch {};
        restartWatch(init, exe, &watch);
        if (ready == 0) {
            if (opened.store.takeCaptureRequest() catch false) {
                captureCurrentOffer(allocator, init, opened.cfg, &opened.store);
            }
            continue;
        }
        acceptPolledClients(allocator, init, &opened, polls, &clients, &server);
    }
}

fn acceptPolledClients(
    allocator: std.mem.Allocator,
    init: std.process.Init,
    opened: anytype,
    polls: []const std.posix.pollfd,
    clients: *std.ArrayList(Client),
    server: *std.Io.net.Server,
) void {
    const polled_clients = clients.items.len;
    if (polls.len > 0 and polls[0].revents & std.posix.POLL.IN != 0) {
        if (server.accept(init.io)) |stream| {
            clients.append(allocator, .{ .fd = stream.socket.handle, .pending = .empty }) catch {
                _ = std.posix.system.close(stream.socket.handle);
            };
        } else |_| {}
    }
    var index: usize = 0;
    while (index < polled_clients) {
        if (index >= clients.items.len) break;
        const poll_index = index + 1;
        if (poll_index >= polls.len or polls[poll_index].revents & std.posix.POLL.IN == 0) {
            index += 1;
            continue;
        }
        if (readClient(allocator, init, opened, &clients.items[index])) {
            index += 1;
        } else {
            _ = std.posix.system.close(clients.items[index].fd);
            clients.items[index].pending.deinit(allocator);
            // swapRemove moves the last client into this slot. That client was
            // polled at the old last index, so this slot's revents belong to
            // the fd just closed. Reading the moved fd here blocks when it is idle.
            _ = clients.swapRemove(index);
            index += 1;
        }
    }
}

fn readClient(allocator: std.mem.Allocator, init: std.process.Init, opened: anytype, client: *Client) bool {
    var buf: [8192]u8 = undefined;
    const n = std.posix.read(client.fd, &buf) catch return false;
    if (n == 0) return false;
    client.pending.appendSlice(allocator, buf[0..n]) catch return false;
    while (framedRequestSpan(client.pending.items)) |span| {
        const request = client.pending.items[span.body_start..][0..span.body_len];
        const response = core.rpc.handleOpened(allocator, init, request, opened) catch return false;
        defer allocator.free(response);
        const framed = core.rpc.frame(allocator, response) catch return false;
        defer allocator.free(framed);
        writeAllFd(client.fd, framed) catch return false;
        client.pending.replaceRange(allocator, 0, span.body_start + span.body_len, &.{}) catch return false;
    }
    return true;
}

fn writeAllFd(fd: std.posix.fd_t, bytes: []const u8) !void {
    var offset: usize = 0;
    while (offset < bytes.len) {
        const rc = std.posix.system.write(fd, bytes.ptr + offset, bytes.len - offset);
        switch (std.posix.errno(rc)) {
            .SUCCESS => {
                const n: usize = @intCast(rc);
                if (n == 0) return error.WriteFailed;
                offset += n;
            },
            .INTR => continue,
            else => return error.WriteFailed,
        }
    }
}

fn refuseOpenRuntimeDir(allocator: std.mem.Allocator, runtime: []const u8) !void {
    const runtime_z = try allocator.dupeZ(u8, runtime);
    defer allocator.free(runtime_z);
    var stat_buf: std.os.linux.Statx = std.mem.zeroes(std.os.linux.Statx);
    const rc = std.os.linux.statx(std.os.linux.AT.FDCWD, runtime_z, std.os.linux.AT.STATX_SYNC_AS_STAT, std.os.linux.STATX.BASIC_STATS, &stat_buf);
    switch (std.posix.errno(rc)) {
        .SUCCESS => {},
        else => return error.Unexpected,
    }
    if (!core.policy.runtimeDirAcceptable(@intCast(stat_buf.uid), @as(u32, stat_buf.mode), @intCast(std.os.linux.getuid()))) return error.Unexpected;
}

fn rpcSocketPath(allocator: std.mem.Allocator, init: std.process.Init) ![]u8 {
    const runtime = init.environ_map.*.get("XDG_RUNTIME_DIR") orelse return error.MissingRuntimeDir;
    return std.fmt.allocPrint(allocator, "{s}/ditox/rpc.sock", .{runtime});
}

fn listenRpc(allocator: std.mem.Allocator, init: std.process.Init, path: []const u8) !std.Io.net.Server {
    const runtime = init.environ_map.*.get("XDG_RUNTIME_DIR") orelse return error.MissingRuntimeDir;
    try refuseOpenRuntimeDir(allocator, runtime);
    if (std.mem.lastIndexOfScalar(u8, path, '/')) |slash| {
        const dir_z = try allocator.dupeZ(u8, path[0..slash]);
        defer allocator.free(dir_z);
        switch (std.posix.errno(std.posix.system.mkdir(dir_z, 0o700))) {
            .SUCCESS, .EXIST => {},
            else => return error.Unexpected,
        }
    }
    const path_z = try allocator.dupeZ(u8, path);
    defer allocator.free(path_z);
    _ = std.posix.system.unlink(path_z);
    const address = try std.Io.net.UnixAddress.init(path);
    const server = try address.listen(init.io, .{});
    if (std.posix.system.fchmod(server.socket.handle, core.policy.socket_mode) != 0) return error.Unexpected;
    return server;
}

fn watchArgv(exe: []const u8) [5][]const u8 {
    // wl-clipboard 2.3 treats a "--" after --watch as an error and exits.
    return .{ "wl-paste", "--watch", exe, "ingest", "wake" };
}

fn spawnWatch(allocator: std.mem.Allocator, init: std.process.Init, exe: []const u8, argv: []const []const u8) !std.process.Child {
    _ = allocator;
    _ = exe;
    return std.process.spawn(init.io, .{
        .argv = argv,
        .stdin = .ignore,
        .stdout = .ignore,
        .stderr = .ignore,
    });
}

fn restartWatch(init: std.process.Init, exe: []const u8, watch: *std.process.Child) void {
    const pid = watch.id orelse return;
    var status: c_int = 0;
    const waited = std.posix.system.waitpid(pid, &status, 1);
    if (waited == 0) return;
    watch.id = null;
    const argv = watchArgv(exe);
    watch.* = spawnWatch(init.gpa, init, exe, &argv) catch return;
}

fn ingestWake(allocator: std.mem.Allocator, init: std.process.Init) !void {
    const drained = core.util.readAllStdin(allocator) catch return;
    allocator.free(drained);
    const sensitive = core.policy.offerIsSensitive(init.environ_map.*.get("CLIPBOARD_STATE"));
    const body = try std.fmt.allocPrint(allocator, "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"clipboard.wake\",\"params\":{{\"sensitive\":{s}}}}}", .{if (sensitive) "true" else "false"});
    defer allocator.free(body);
    try sendRpc(allocator, init, body);
}

fn captureCurrentOffer(allocator: std.mem.Allocator, init: std.process.Init, cfg: core.config.Config, store: *core.storage.Storage) void {
    if (store.isWatcherPaused() catch true) return;
    const listed = core.clipboard.listMimeTypes(allocator, init) catch return;
    defer listed.deinit(allocator);
    const types = switch (listed) {
        .types => |value| value,
        .empty => return,
        .failed => |detail| {
            allocator.free(detail);
            return;
        },
    };

    var image_listed = false;
    var image_ok = false;
    var parts: std.ArrayList(core.policy.OfferPart) = .empty;
    defer {
        for (parts.items) |part| {
            allocator.free(part.mime);
            allocator.free(part.bytes);
        }
        parts.deinit(allocator);
    }
    for (types) |mime| {
        if (!core.policy.mimeAllowed(mime)) continue;
        const base = core.policy.mimeBase(mime);
        const is_image = std.mem.startsWith(u8, base, "image/");
        if (is_image) image_listed = true;
        const read = core.clipboard.readOffer(allocator, init, mime) catch continue;
        switch (read) {
            .bytes => |bytes| {
                if (is_image and core.util.detectImageMime(bytes) == null) {
                    allocator.free(bytes);
                    continue;
                }
                const owned_mime = allocator.dupe(u8, base) catch {
                    allocator.free(bytes);
                    continue;
                };
                if (is_image) image_ok = true;
                parts.append(allocator, .{ .mime = owned_mime, .bytes = bytes }) catch {
                    allocator.free(owned_mime);
                    allocator.free(bytes);
                    continue;
                };
            },
            else => read.deinit(allocator),
        }
    }
    if (core.policy.dropUnreadableImage(image_listed, image_ok)) return;
    if (parts.items.len == 0) return;
    _ = core.app.storeOffer(allocator, init, cfg, store, parts.items) catch {};
}

fn sendRpc(allocator: std.mem.Allocator, init: std.process.Init, body: []const u8) !void {
    const framed = try core.rpc.frame(allocator, body);
    defer allocator.free(framed);
    const socket_path = try rpcSocketPath(allocator, init);
    defer allocator.free(socket_path);
    const address = std.Io.net.UnixAddress.init(socket_path) catch return;
    const stream = address.connect(init.io) catch return;
    defer stream.close(init.io);
    try writeAllFd(stream.socket.handle, framed);
    var buf: [1024]u8 = undefined;
    _ = std.posix.read(stream.socket.handle, &buf) catch {};
}

fn ingestStdin(allocator: std.mem.Allocator, init: std.process.Init, kind: []const u8) !void {
    if (core.policy.offerIsSensitive(init.environ_map.*.get("CLIPBOARD_STATE"))) return;
    const bytes = try core.util.readAllStdin(allocator);
    defer allocator.free(bytes);
    if (bytes.len == 0) return;
    const encoded_len = std.base64.standard.Encoder.calcSize(bytes.len);
    const encoded = try allocator.alloc(u8, encoded_len);
    defer allocator.free(encoded);
    _ = std.base64.standard.Encoder.encode(encoded, bytes);
    const body = try std.fmt.allocPrint(allocator, "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"clipboard.ingest\",\"params\":{{\"kind\":\"{s}\",\"data\":\"{s}\"}}}}", .{ kind, encoded });
    defer allocator.free(body);
    const framed = try core.rpc.frame(allocator, body);
    defer allocator.free(framed);

    const socket_path = try rpcSocketPath(allocator, init);
    defer allocator.free(socket_path);
    const address = std.Io.net.UnixAddress.init(socket_path) catch return;
    const stream = address.connect(init.io) catch return;
    defer stream.close(init.io);
    try writeAllFd(stream.socket.handle, framed);
    var buf: [1024]u8 = undefined;
    _ = std.posix.read(stream.socket.handle, &buf) catch {};
}

/// Shared capture loop used by both legacy `watch` and the new single-owner `daemon`.
/// All heavy DB writes (add*, markWatcherSeen) go through the caller's store.
/// Capture failures must never terminate the loop: a transient SQLite-busy or
/// disk error on one tick is logged and surfaced through watcher.status
/// (`last_error`), and the next poll retries.
fn runCaptureLoop(
    allocator: std.mem.Allocator,
    init: std.process.Init,
    cfg: core.config.Config,
    store: *core.storage.Storage,
) !void {
    var last_hash: ?[64]u8 = null;
    var had_error = false;
    // A previous daemon run may have died with an error recorded; this run
    // starts fresh.
    store.clearWatcherError() catch {};
    while (true) {
        store.markWatcherSeen() catch {};

        if (store.isWatcherPaused() catch false) {
            try std.Io.sleep(init.io, std.Io.Duration.fromMilliseconds(cfg.poll_interval_ms), .awake);
            continue;
        }

        if (isActiveWindowExcluded(allocator, init, cfg) catch false) {
            try std.Io.sleep(init.io, std.Io.Duration.fromMilliseconds(cfg.poll_interval_ms), .awake);
            continue;
        }

        var tick_error = false;
        // An announced image that cannot be read is retried next poll. It is
        // not stored as the text alternative, and it is not a watcher error.
        // Only a real wl-paste or storage failure is recorded.
        if (captureImage(allocator, init, cfg, store, &last_hash)) |image_step| {
            switch (image_step) {
                .done => {},
                .read_text => {
                    if (captureText(allocator, init, cfg, store, &last_hash)) |text_step| {
                        switch (text_step) {
                            .done, .read_text => {},
                            .failed => |reason| {
                                defer allocator.free(reason);
                                tick_error = true;
                                reportCaptureError(allocator, store, "text capture", reason);
                            },
                        }
                    } else |err| {
                        tick_error = true;
                        reportCaptureError(allocator, store, "text capture", @errorName(err));
                    }
                },
                .failed => |reason| {
                    defer allocator.free(reason);
                    tick_error = true;
                    reportCaptureError(allocator, store, "image capture", reason);
                },
            }
        } else |err| {
            tick_error = true;
            reportCaptureError(allocator, store, "image capture", @errorName(err));
        }
        if (had_error and !tick_error) store.clearWatcherError() catch {};
        had_error = tick_error;

        try std.Io.sleep(init.io, std.Io.Duration.fromMilliseconds(cfg.poll_interval_ms), .awake);
    }
}

fn reportCaptureError(allocator: std.mem.Allocator, store: *core.storage.Storage, context: []const u8, reason: []const u8) void {
    std.log.warn("{s} failed: {s}", .{ context, reason });
    const message = std.fmt.allocPrint(allocator, "{s} failed: {s}", .{ context, reason }) catch return;
    defer allocator.free(message);
    store.setWatcherError(message) catch {};
}

test "an announced image that cannot be read does not fall through to text" {
    try std.testing.expectEqual(.try_untyped, afterAnnouncedImage(.unavailable));
    try std.testing.expectEqual(.try_untyped, afterAnnouncedImage(.empty));
    try std.testing.expectEqual(.try_untyped, afterTypedImageBytes(false));
    try std.testing.expectEqual(.skip, afterUntypedFallback(.unavailable, false));
    try std.testing.expectEqual(.skip, afterUntypedFallback(.empty, false));
    try std.testing.expectEqual(.skip, afterUntypedFallback(.bytes, false));
    try std.testing.expectEqual(.store, afterUntypedFallback(.bytes, true));
    try std.testing.expectEqual(.fail, afterAnnouncedImage(.failed));
    try std.testing.expectEqual(.store, afterTypedImageBytes(true));
}

test "an empty clipboard and a refused text type are not watcher failures" {
    try std.testing.expectEqual(.skip, afterTextOffer(.empty));
    try std.testing.expectEqual(.skip, afterTextOffer(.unavailable));
    try std.testing.expectEqual(.fail, afterTextOffer(.failed));
    try std.testing.expectEqual(.store, afterTextOffer(.bytes));
}

fn testingInit(env: *std.process.Environ.Map) std.process.Init {
    return .{
        .minimal = undefined,
        .arena = undefined,
        .gpa = std.testing.allocator,
        .io = std.testing.io,
        .environ_map = env,
        .preopens = undefined,
    };
}

test "a group-writable runtime directory is refused" {
    const allocator = std.testing.allocator;
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    const runtime = try std.fmt.allocPrint(allocator, ".zig-cache/tmp/{s}", .{tmp.sub_path});
    defer allocator.free(runtime);
    const runtime_z = try allocator.dupeZ(u8, runtime);
    defer allocator.free(runtime_z);
    try std.testing.expect(std.posix.errno(std.os.linux.chmod(runtime_z, 0o777)) == .SUCCESS);

    var env = std.process.Environ.Map.init(allocator);
    defer env.deinit();
    try env.put("XDG_RUNTIME_DIR", runtime);
    const init = testingInit(&env);
    const socket_path = try std.fmt.allocPrint(allocator, "{s}/ditox/rpc.sock", .{runtime});
    defer allocator.free(socket_path);
    try std.testing.expectError(error.Unexpected, listenRpc(allocator, init, socket_path));

    try std.testing.expect(std.posix.errno(std.os.linux.chmod(runtime_z, 0o700)) == .SUCCESS);
    var server = try listenRpc(allocator, init, socket_path);
    defer server.deinit(std.testing.io);
}

test "accepting the first connection does not panic" {
    const allocator = std.testing.allocator;
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    const root = try std.fmt.allocPrint(allocator, ".zig-cache/tmp/{s}", .{tmp.sub_path});
    defer allocator.free(root);
    const runtime = try std.fmt.allocPrint(allocator, "{s}/run", .{root});
    defer allocator.free(runtime);
    try std.Io.Dir.cwd().createDirPath(std.testing.io, runtime);
    const runtime_z = try allocator.dupeZ(u8, runtime);
    defer allocator.free(runtime_z);
    try std.testing.expect(std.posix.errno(std.os.linux.chmod(runtime_z, 0o700)) == .SUCCESS);

    var env = std.process.Environ.Map.init(allocator);
    defer env.deinit();
    try env.put("HOME", root);
    try env.put("XDG_CONFIG_HOME", root);
    try env.put("XDG_DATA_HOME", root);
    try env.put("XDG_RUNTIME_DIR", runtime);
    const init = testingInit(&env);
    var opened = try core.app.openStore(allocator, init);
    defer opened.cfg.deinit();
    defer opened.store.close();

    const socket_path = try std.fmt.allocPrint(allocator, "{s}/ditox/rpc.sock", .{runtime});
    defer allocator.free(socket_path);
    var server = try listenRpc(allocator, init, socket_path);
    defer server.deinit(std.testing.io);

    const address = try std.Io.net.UnixAddress.init(socket_path);
    const stream = try address.connect(std.testing.io);
    defer stream.close(std.testing.io);

    var polls = [_]std.posix.pollfd{
        .{ .fd = server.socket.handle, .events = std.posix.POLL.IN, .revents = 0 },
    };
    try std.testing.expect(try std.posix.poll(&polls, 1000) >= 1);
    var clients: std.ArrayList(Client) = .empty;
    defer {
        for (clients.items) |*client| {
            _ = std.posix.system.close(client.fd);
            client.pending.deinit(allocator);
        }
        clients.deinit(allocator);
    }
    acceptPolledClients(allocator, init, &opened, &polls, &clients, &server);
    try std.testing.expectEqual(@as(usize, 1), clients.items.len);
}

test "closing one of two clients returns while the other stays idle" {
    const allocator = std.testing.allocator;
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    const root = try std.fmt.allocPrint(allocator, ".zig-cache/tmp/{s}", .{tmp.sub_path});
    defer allocator.free(root);
    const runtime = try std.fmt.allocPrint(allocator, "{s}/run", .{root});
    defer allocator.free(runtime);
    try std.Io.Dir.cwd().createDirPath(std.testing.io, runtime);
    const runtime_z = try allocator.dupeZ(u8, runtime);
    defer allocator.free(runtime_z);
    try std.testing.expect(std.posix.errno(std.os.linux.chmod(runtime_z, 0o700)) == .SUCCESS);

    var env = std.process.Environ.Map.init(allocator);
    defer env.deinit();
    try env.put("HOME", root);
    try env.put("XDG_CONFIG_HOME", root);
    try env.put("XDG_DATA_HOME", root);
    try env.put("XDG_RUNTIME_DIR", runtime);
    const init = testingInit(&env);
    var opened = try core.app.openStore(allocator, init);
    defer opened.cfg.deinit();
    defer opened.store.close();

    const socket_path = try std.fmt.allocPrint(allocator, "{s}/ditox/rpc.sock", .{runtime});
    defer allocator.free(socket_path);
    var server = try listenRpc(allocator, init, socket_path);
    defer server.deinit(std.testing.io);
    const address = try std.Io.net.UnixAddress.init(socket_path);

    const first = try address.connect(std.testing.io);
    var polls = [_]std.posix.pollfd{
        .{ .fd = server.socket.handle, .events = std.posix.POLL.IN, .revents = 0 },
        .{ .fd = -1, .events = std.posix.POLL.IN, .revents = 0 },
        .{ .fd = -1, .events = std.posix.POLL.IN, .revents = 0 },
    };
    try std.testing.expect(try std.posix.poll(polls[0..1], 1000) >= 1);
    var clients: std.ArrayList(Client) = .empty;
    defer {
        for (clients.items) |*client| {
            _ = std.posix.system.close(client.fd);
            client.pending.deinit(allocator);
        }
        clients.deinit(allocator);
    }
    acceptPolledClients(allocator, init, &opened, polls[0..1], &clients, &server);
    try std.testing.expectEqual(@as(usize, 1), clients.items.len);

    const second = try address.connect(std.testing.io);
    defer second.close(std.testing.io);
    polls[0].revents = 0;
    polls[1] = .{ .fd = clients.items[0].fd, .events = std.posix.POLL.IN, .revents = 0 };
    try std.testing.expect(try std.posix.poll(polls[0..2], 1000) >= 1);
    acceptPolledClients(allocator, init, &opened, polls[0..2], &clients, &server);
    try std.testing.expectEqual(@as(usize, 2), clients.items.len);
    const idle_fd = clients.items[1].fd;

    first.close(std.testing.io);
    polls[1] = .{ .fd = clients.items[0].fd, .events = std.posix.POLL.IN, .revents = 0 };
    polls[2] = .{ .fd = idle_fd, .events = std.posix.POLL.IN, .revents = 0 };
    try std.testing.expect(try std.posix.poll(&polls, 1000) >= 1);
    try std.testing.expect(polls[1].revents & std.posix.POLL.IN != 0);
    try std.testing.expect(polls[2].revents & std.posix.POLL.IN == 0);
    acceptPolledClients(allocator, init, &opened, &polls, &clients, &server);
    try std.testing.expectEqual(@as(usize, 1), clients.items.len);
    try std.testing.expectEqual(idle_fd, clients.items[0].fd);
}
