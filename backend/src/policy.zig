const std = @import("std");

pub const WindowSnap = struct {
    class: []const u8,
    title: []const u8,
};

pub const CaptureFacts = struct {
    kind: []const u8,
    bytes: []const u8,
    image_mime: ?[]const u8 = null,
    window: ?WindowSnap = null,
    content_hash: []const u8,
    self_hash: ?[]const u8 = null,
    excluded_apps: []const []const u8 = &.{},
    excluded_windows: []const []const u8 = &.{},
    secret_patterns: []const []const u8 = &.{},
};

pub const CaptureDecision = struct {
    store: bool,
    reason: []const u8,
    source_app: ?[]const u8 = null,
};

pub const max_image_read_bytes: usize = 128 * 1024 * 1024;

pub fn offerIsSensitive(state: ?[]const u8) bool {
    const value = state orelse return false;
    return std.mem.eql(u8, value, "sensitive");
}

pub fn wakeStores(sensitive: bool) bool {
    return !sensitive;
}

pub fn dropUnreadableImage(image_listed: bool, image_ok: bool) bool {
    return image_listed and !image_ok;
}

pub fn runtimeDirAcceptable(owner: u32, mode: u32, user: u32) bool {
    if (owner != user) return false;
    if (mode & 0o022 != 0) return false;
    return true;
}

pub const socket_mode: u32 = 0o600;

pub fn mimeBase(mime: []const u8) []const u8 {
    const trimmed = std.mem.trim(u8, mime, " \t\r\n");
    if (std.mem.indexOfScalar(u8, trimmed, ';')) |index| {
        return std.mem.trim(u8, trimmed[0..index], " \t");
    }
    return trimmed;
}

pub fn mimeAllowed(mime: []const u8) bool {
    const base = mimeBase(mime);
    return std.mem.eql(u8, base, "text/plain") or
        std.mem.eql(u8, base, "text/html") or
        std.mem.eql(u8, base, "text/uri-list") or
        std.mem.eql(u8, base, "image/png") or
        std.mem.eql(u8, base, "image/jpeg");
}

pub const OfferPart = struct {
    mime: []const u8,
    bytes: []const u8,
};

pub const PartRole = enum { primary, sibling };

pub const PlannedPart = struct {
    index: usize,
    role: PartRole,
};

pub const OfferPlan = struct {
    kind: []const u8,
    primary: usize,
    preview: ?usize = null,
    kept: []const PlannedPart,
};

pub const OfferOutcome = union(enum) {
    empty,
    secret,
    store: OfferPlan,
};

/// `kept_out` must hold at least one slot per input. The returned plan points into it.
pub fn planOffer(parts: []const OfferPart, secret_patterns: []const []const u8, kept_out: []PlannedPart) OfferOutcome {
    var plain: ?usize = null;
    var html: ?usize = null;
    var uri: ?usize = null;
    var png: ?usize = null;
    var jpeg: ?usize = null;
    for (parts, 0..) |part, index| {
        if (!mimeAllowed(part.mime) or part.bytes.len == 0) continue;
        const base = mimeBase(part.mime);
        if (std.mem.eql(u8, base, "text/plain") and plain == null) plain = index;
        if (std.mem.eql(u8, base, "text/html") and html == null) html = index;
        if (std.mem.eql(u8, base, "text/uri-list") and uri == null) uri = index;
        if (std.mem.eql(u8, base, "image/png") and png == null) png = index;
        if (std.mem.eql(u8, base, "image/jpeg") and jpeg == null) jpeg = index;
        if (std.mem.startsWith(u8, base, "text/") and matchesSecret(part.bytes, secret_patterns)) return .secret;
    }

    const image = png orelse jpeg;
    const text_primary = plain orelse html orelse uri;
    if (image == null and text_primary == null) return .empty;

    var count: usize = 0;
    const primary = image orelse text_primary.?;
    const kind: []const u8 = if (image != null) "image" else "text";
    kept_out[count] = .{ .index = primary, .role = .primary };
    count += 1;

    const text_slots = [_] ?usize{ plain, html, uri };
    for (text_slots) |slot| {
        const index = slot orelse continue;
        if (index == primary) continue;
        kept_out[count] = .{ .index = index, .role = .sibling };
        count += 1;
    }

    const preview = if (image != null) plain orelse uri else null;
    return .{ .store = .{
        .kind = kind,
        .primary = primary,
        .preview = preview,
        .kept = kept_out[0..count],
    } };
}

pub fn capUtf8(text: []const u8, max_bytes: u32) []const u8 {
    if (max_bytes == 0 or text.len <= max_bytes) return text;
    var end: usize = max_bytes;
    while (end > 0 and text[end] & 0xC0 == 0x80) end -= 1;
    return text[0..end];
}

pub fn decideCapture(facts: CaptureFacts) CaptureDecision {
    if (facts.bytes.len == 0) return .{ .store = false, .reason = "empty" };
    if (windowExcluded(facts.window, facts.excluded_apps, facts.excluded_windows)) {
        return .{ .store = false, .reason = "excluded" };
    }
    if (std.mem.eql(u8, facts.kind, "image") and facts.image_mime == null) {
        return .{ .store = false, .reason = "not-image" };
    }
    if (!std.mem.eql(u8, facts.kind, "image") and matchesSecret(facts.bytes, facts.secret_patterns)) {
        return .{ .store = false, .reason = "secret" };
    }
    if (facts.self_hash) |saved| {
        if (std.mem.eql(u8, saved, facts.content_hash)) return .{ .store = false, .reason = "self" };
    }
    return .{ .store = true, .reason = "", .source_app = sourceClass(facts.window) };
}

fn sourceClass(window: ?WindowSnap) ?[]const u8 {
    const snap = window orelse return null;
    if (snap.class.len == 0) return null;
    return snap.class;
}

fn windowExcluded(window: ?WindowSnap, apps: []const []const u8, titles: []const []const u8) bool {
    const snap = window orelse return false;
    return containsIgnoreCase(snap.class, apps) or containsIgnoreCase(snap.title, titles);
}

fn containsIgnoreCase(value: []const u8, patterns: []const []const u8) bool {
    for (patterns) |pattern| {
        if (pattern.len == 0 or pattern.len > value.len) continue;
        var start: usize = 0;
        while (start + pattern.len <= value.len) : (start += 1) {
            for (pattern, 0..) |needle, offset| {
                if (std.ascii.toLower(value[start + offset]) != std.ascii.toLower(needle)) break;
            } else return true;
        }
    }
    return false;
}

fn matchesSecret(text: []const u8, patterns: []const []const u8) bool {
    for (patterns) |pattern| {
        if (patternMatches(text, pattern)) return true;
    }
    return false;
}

fn patternMatches(text: []const u8, pattern: []const u8) bool {
    const open = std.mem.indexOfScalar(u8, pattern, '[') orelse return std.mem.indexOf(u8, text, pattern) != null;
    const close = std.mem.indexOfScalarPos(u8, pattern, open, ']') orelse return std.mem.indexOf(u8, text, pattern) != null;
    if (close + 1 >= pattern.len or pattern[close + 1] != '{') return std.mem.indexOf(u8, text, pattern) != null;
    const end = std.mem.indexOfScalarPos(u8, pattern, close + 2, '}') orelse return false;
    const count = std.fmt.parseUnsigned(usize, pattern[close + 2 .. end], 10) catch return false;
    const prefix = pattern[0..open];
    const class = pattern[open + 1 .. close];
    if (prefix.len + count > text.len) return false;
    var index: usize = 0;
    while (index + prefix.len + count <= text.len) : (index += 1) {
        if (!std.mem.eql(u8, text[index .. index + prefix.len], prefix)) continue;
        if (allInClass(text[index + prefix.len .. index + prefix.len + count], class)) return true;
    }
    return false;
}

fn allInClass(bytes: []const u8, class: []const u8) bool {
    if (bytes.len == 0) return false;
    for (bytes) |byte| if (!byteInClass(byte, class)) return false;
    return true;
}

fn byteInClass(byte: u8, class: []const u8) bool {
    var index: usize = 0;
    while (index < class.len) : (index += 1) {
        if (index + 2 < class.len and class[index + 1] == '-') {
            if (byte >= class[index] and byte <= class[index + 2]) return true;
            index += 2;
            continue;
        }
        if (byte == class[index]) return true;
    }
    return false;
}

const github = "ghp_0123456789abcdef";
const aws = "AKIAIOSFODNN7EXAMPLE";

test "an offer keeps one image and its text siblings" {
    const parts = [_]OfferPart{
        .{ .mime = "text/plain;charset=utf-8", .bytes = "https://example.com/a" },
        .{ .mime = "text/html", .bytes = "<p>https://example.com/a</p>" },
        .{ .mime = "image/jpeg", .bytes = "jpeg" },
        .{ .mime = "image/png", .bytes = "png" },
        .{ .mime = "application/x-private", .bytes = "nope" },
    };
    var kept: [5]PlannedPart = undefined;
    const outcome = planOffer(&parts, &.{ "ghp_", "AKIA[A-Z0-9]{16}" }, &kept);
    const plan = outcome.store;
    try std.testing.expectEqualStrings("image", plan.kind);
    try std.testing.expectEqual(@as(usize, 3), plan.primary);
    try std.testing.expectEqual(@as(?usize, 0), plan.preview);
    try std.testing.expectEqual(@as(usize, 3), plan.kept.len);
}

test "a secret in a text sibling drops the whole offer" {
    const parts = [_]OfferPart{
        .{ .mime = "image/png", .bytes = "png" },
        .{ .mime = "text/html", .bytes = "token ghp_0123456789abcdef" },
    };
    var kept: [2]PlannedPart = undefined;
    const outcome = planOffer(&parts, &.{ "ghp_", "AKIA[A-Z0-9]{16}" }, &kept);
    try std.testing.expect(outcome == .secret);
}

test "html without plain text stays a text row" {
    const parts = [_]OfferPart{
        .{ .mime = "text/html", .bytes = "<p>hola</p>" },
    };
    var kept: [1]PlannedPart = undefined;
    const plan = planOffer(&parts, &.{}, &kept).store;
    try std.testing.expectEqualStrings("text", plan.kind);
    try std.testing.expectEqual(@as(usize, 0), plan.primary);
    try std.testing.expect(plan.preview == null);
}

test "a sensitive offer stores nothing" {
    try std.testing.expect(offerIsSensitive("sensitive"));
    try std.testing.expect(!wakeStores(true));
    try std.testing.expect(wakeStores(false));
}

test "an announced image that cannot be read is not replaced by text" {
    try std.testing.expect(dropUnreadableImage(true, false));
    try std.testing.expect(!dropUnreadableImage(true, true));
    try std.testing.expect(!dropUnreadableImage(false, false));
}

test "image reads up to the retention cap are allowed" {
    try std.testing.expect(max_image_read_bytes >= 128 * 1024 * 1024);
    try std.testing.expectEqual(@as(u32, 0o600), socket_mode);
    try std.testing.expect(!runtimeDirAcceptable(1, 0o700, 2));
    try std.testing.expect(!runtimeDirAcceptable(1, 0o702, 1));
    try std.testing.expect(runtimeDirAcceptable(1, 0o700, 1));
}

test "text/plain charset parameter is plain text" {
    const parts = [_]OfferPart{.{ .mime = "text/plain;charset=utf-8", .bytes = "hola" }};
    var kept: [1]PlannedPart = undefined;
    const plan = planOffer(&parts, &.{}, &kept).store;
    try std.testing.expectEqualStrings("text", plan.kind);
    try std.testing.expectEqualStrings("text/plain", mimeBase(parts[plan.primary].mime));
}

test "a private clipboard type is rejected" {
    const parts = [_]OfferPart{.{ .mime = "application/x-private", .bytes = "secret-bytes" }};
    var kept: [1]PlannedPart = undefined;
    try std.testing.expect(planOffer(&parts, &.{}, &kept) == .empty);
    try std.testing.expect(!mimeAllowed("application/x-private"));
}

test "only a sensitive clipboard offer is dropped before ingest" {
    try std.testing.expect(offerIsSensitive("sensitive"));
    try std.testing.expect(!offerIsSensitive("data"));
    try std.testing.expect(!offerIsSensitive("clear"));
    try std.testing.expect(!offerIsSensitive(null));
}

test "plain text from a normal window is stored with that class" {
    const decision = decideCapture(.{
        .kind = "text",
        .bytes = "https://example.com",
        .content_hash = "abc",
        .window = .{ .class = "firefox", .title = "Example" },
        .secret_patterns = &.{ "ghp_", "AKIA[A-Z0-9]{16}" },
    });
    try std.testing.expect(decision.store);
    try std.testing.expectEqualStrings("firefox", decision.source_app.?);
}

test "an excluded window is not stored" {
    const decision = decideCapture(.{
        .kind = "text",
        .bytes = "hunter2",
        .content_hash = "abc",
        .window = .{ .class = "org.keepassxc.KeePassXC", .title = "Unlock" },
        .excluded_apps = &.{ "KeePassXC", "Bitwarden" },
    });
    try std.testing.expect(!decision.store);
    try std.testing.expectEqualStrings("excluded", decision.reason);
}

test "a github token is not stored and an empty pattern list keeps it" {
    const blocked = decideCapture(.{
        .kind = "text",
        .bytes = github,
        .content_hash = "abc",
        .secret_patterns = &.{ "ghp_", "github_pat_" },
    });
    try std.testing.expect(!blocked.store);
    try std.testing.expectEqualStrings("secret", blocked.reason);

    const kept = decideCapture(.{
        .kind = "text",
        .bytes = github,
        .content_hash = "abc",
        .secret_patterns = &.{},
    });
    try std.testing.expect(kept.store);
}

test "an aws access key shape is not stored" {
    const decision = decideCapture(.{
        .kind = "text",
        .bytes = "export AWS_ACCESS_KEY_ID=" ++ aws,
        .content_hash = "abc",
        .secret_patterns = &.{"AKIA[A-Z0-9]{16}"},
    });
    try std.testing.expect(!decision.store);
    try std.testing.expectEqualStrings("secret", decision.reason);
}

test "a private key block is not stored" {
    const decision = decideCapture(.{
        .kind = "text",
        .bytes = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n",
        .content_hash = "abc",
        .secret_patterns = &.{"-----BEGIN OPENSSH PRIVATE KEY-----"},
    });
    try std.testing.expectEqualStrings("secret", decision.reason);
}

test "the same hash as our own paste is skipped" {
    const decision = decideCapture(.{
        .kind = "text",
        .bytes = "pasted",
        .content_hash = "same",
        .self_hash = "same",
    });
    try std.testing.expectEqualStrings("self", decision.reason);
}

test "text cap keeps whole codepoints and a second copy of the same prefix stays the same length" {
    const long = "ááááá"; // each á is 2 bytes in UTF-8
    const capped = capUtf8(long, 3);
    try std.testing.expectEqual(@as(usize, 2), capped.len);
    try std.testing.expectEqualStrings("á", capped);
    try std.testing.expectEqual(@as(usize, 0), capUtf8(long, 1).len);
}

test "image bytes without a sniffed type are not stored" {
    const decision = decideCapture(.{
        .kind = "image",
        .bytes = "not-an-image",
        .image_mime = null,
        .content_hash = "abc",
    });
    try std.testing.expectEqualStrings("not-image", decision.reason);
}
