import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deflateSync } from "node:zlib";

const repoRoot = join(import.meta.dir, "..");
const ditox = join(repoRoot, "zig-out", "bin", "ditox");
const ditoxd = join(repoRoot, "zig-out", "bin", "ditoxd");

let tempDir = "";
let env: Record<string, string> = {};

beforeEach(() => {
  tempDir = mkdtempSync(join(tmpdir(), "ditox-cli-"));
  env = {
    DITOX_DATA_DIR: join(tempDir, "data"),
    DITOX_CONFIG: join(tempDir, "config.toml"),
    DITOX_CLIPBOARD_MOCK: join(tempDir, "clipboard.txt"),
  };
});

afterEach(() => {
  if (tempDir && existsSync(tempDir)) rmSync(tempDir, { recursive: true, force: true });
});

describe("ditox CLI smoke", () => {
  test("covers add list search print copy paste favorite delete clear status and repair", () => {
    expect(run(["add", "alpha searchable"])).toBe("1");
    expect(run(["add", "beta second"])).toBe("2");
    expect(run(["print", "1"])).toBe("alpha searchable");

    const listed = JSON.parse(run(["list", "--query", "searchable"]));
    expect(listed.entries).toHaveLength(1);
    expect(listed.entries[0].content).toBe("alpha searchable");

    run(["copy", "1"]);
    expect(readFileSync(env.DITOX_CLIPBOARD_MOCK!, "utf8")).toBe("alpha searchable");

    run(["paste", "2"]);
    expect(readFileSync(env.DITOX_CLIPBOARD_MOCK!, "utf8")).toBe("beta second");

    expect(run(["favorite", "1"])).toBe("true");
    let status = JSON.parse(run(["status"]));
    expect(status.stats.favorites).toBe(1);

    expect(run(["delete", "2"])).toBe("true");
    expect(run(["add", "clear me without touching pinned"])).toBe("3");
    expect(run(["clear", "text", "--keep-pinned"])).toBe("1");
    status = JSON.parse(run(["status"]));
    expect(status.stats.entries).toBe(1);
    expect(status.stats.favorites).toBe(1);
    expect(run(["clear", "text"])).toBe("1");

    expect(run(["add", "gamma"])).toBe("4");
    expect(run(["clear", "all"])).toBe("1");

    const repair = JSON.parse(run(["repair"]));
    expect(repair.ok).toBe(true);

    expect(run(["pause", "60000"])).toBe("paused_for_ms=60000");
    status = JSON.parse(run(["status"]));
    expect(status.watcher.paused).toBe(true);
    expect(run(["resume"])).toBe("resumed");
    expect(run(["-pause", "2s"])).toBe("paused_for_ms=2000");
    status = JSON.parse(run(["status"]));
    expect(status.watcher.paused).toBe(true);
    expect(run(["--pause=1m"])).toBe("paused_for_ms=60000");
    expect(run(["resume"])).toBe("resumed");
  });

  test("orders searched history by relevance before recency", () => {
    expect(run(["add", "needle"])).toBe("1");
    expect(run(["add", "zzz needle newest"])).toBe("2");
    expect(run(["add", "needle prefix match"])).toBe("3");

    const listed = JSON.parse(run(["list", "--query", "needle"]));
    expect(listed.entries.map((entry: { content: string }) => entry.content)).toEqual([
      "needle",
      "needle prefix match",
      "zzz needle newest",
    ]);
  });

  test("finds fuzzy subsequence search matches", () => {
    expect(run(["add", "network latency error"])).toBe("1");
    expect(run(["add", "totally separate"])).toBe("2");

    const listed = JSON.parse(run(["list", "--query", "nle"]));
    expect(listed.entries.map((entry: { content: string }) => entry.content)).toEqual(["network latency error"]);
  });

  test("accepts backend config aliases for duplicates and history limits", () => {
    env.DITOX_CONFIG = join(tempDir, "compat-aliases.toml");
    writeFileSync(
      env.DITOX_CONFIG,
      [
        "maxHistory = 2",
        "allowDuplicates = true",
        "pollInterval = 25",
        "maxEntryLength = 8",
        "",
      ].join("\n"),
    );

    expect(run(["add", "duplicate value one"])).toBe("1");
    expect(run(["add", "duplicate value one"])).toBe("2");
    expect(run(["add", "newest value"])).toBe("3");

    const status = JSON.parse(run(["status"]));
    expect(status.config.max_entries).toBe(2);
    expect(status.config.allow_duplicates).toBe(true);
    expect(status.config.poll_interval_ms).toBe(25);
    expect(status.config.max_preview_chars).toBe(8);
    expect(status.stats.entries).toBe(2);

    const listed = JSON.parse(run(["list"]));
    expect(listed.entries.map((entry: { id: number }) => entry.id)).toEqual([3, 2]);
    expect(listed.entries[1].preview).toBe("duplicat...");
  });

  test("accepts backend path aliases relative to the config file", () => {
    const configDir = join(tempDir, "compat-config");
    const expectedDb = join(configDir, "history", "clipboard.sqlite");
    const expectedImageDir = join(configDir, "tmp_files");
    mkdirSync(configDir, { recursive: true });
    env.DITOX_CONFIG = join(configDir, "configuration.toml");
    writeFileSync(
      env.DITOX_CONFIG,
      [
        'historyFile = "history/clipboard.sqlite"',
        'tempDir = "tmp_files"',
        "",
      ].join("\n"),
    );

    expect(run(["add", "stored in compat history path"])).toBe("1");
    expect(existsSync(expectedDb)).toBe(true);
    let status = JSON.parse(run(["status"]));
    expect(status.config.db_path).toBe(expectedDb);
    expect(status.stats.entries).toBe(1);

    const png = rgbaPng(1, 1, [[255, 0, 0, 255]]);
    expect(runWithStdin(["--wl-store"], png)).toBe("");
    status = JSON.parse(run(["status"]));
    expect(status.stats.images).toBe(1);

    const listed = JSON.parse(run(["list", "--filter", "images"]));
    expect(listed.entries).toHaveLength(1);
    expect(listed.entries[0].blob_path.startsWith(`${expectedImageDir}/`)).toBe(true);
    expect(existsSync(listed.entries[0].blob_path)).toBe(true);
  });

  test("loads configuration.json backend settings", () => {
    const configDir = join(tempDir, "compat-json");
    const expectedDb = join(configDir, "history", "clipboard.sqlite");
    const expectedImageDir = join(configDir, "tmp_files");
    mkdirSync(configDir, { recursive: true });
    env.DITOX_CONFIG = join(configDir, "configuration.json");
    writeFileSync(
      env.DITOX_CONFIG,
      JSON.stringify(
        {
          allowDuplicates: true,
          historyFile: "history/clipboard.sqlite",
          maxHistory: 2,
          deleteAfter: 3,
          pollInterval: 35,
          maxEntryLength: 7,
          tempDir: "tmp_files",
          excludedApps: ["Bitwarden"],
          excludedWindows: ["Secret Window"],
          autoPaste: {
            enabled: true,
            keybind: "ctrl+shift+v",
            buffer: 4,
          },
        },
        null,
        2,
      ),
    );

    expect(run(["add", "duplicate json value"])).toBe("1");
    expect(run(["add", "duplicate json value"])).toBe("2");
    expect(run(["add", "newest json value"])).toBe("3");

    const status = JSON.parse(run(["status"]));
    expect(status.config.db_path).toBe(expectedDb);
    expect(status.config.image_dir).toBe(expectedImageDir);
    expect(status.config.max_entries).toBe(2);
    expect(status.config.delete_after_seconds).toBe(3);
    expect(status.config.allow_duplicates).toBe(true);
    expect(status.config.poll_interval_ms).toBe(35);
    expect(status.config.max_preview_chars).toBe(7);
    expect(status.config.excluded_apps).toEqual(["Bitwarden"]);
    expect(status.config.excluded_windows).toEqual(["Secret Window"]);
    expect(status.config.auto_paste_enabled).toBe(true);
    expect(status.config.auto_paste_keybind).toBe("ctrl+shift+v");
    expect(status.config.auto_paste_buffer_ms).toBe(4);
    expect(status.stats.entries).toBe(2);
    expect(existsSync(expectedDb)).toBe(true);
    expect(existsSync(expectedImageDir)).toBe(true);

    const listed = JSON.parse(run(["list"]));
    expect(listed.entries.map((entry: { id: number }) => entry.id)).toEqual([3, 2]);
    expect(listed.entries[1].preview).toBe("duplica...");
  });

  test("accepts short CLI aliases for supported operations", () => {
    expect(run(["-v"])).toBe("ditox 0.1.0");
    expect(run(["--version"])).toBe("ditox 0.1.0");
    expect(run(["version"])).toBe("ditox 0.1.0");

    expect(run(["-a", "alias first"])).toBe("1");
    expect(run(["-a", "alias second"])).toBe("2");

    expect(run(["--output-all", "unescaped"]).split("\n").sort()).toEqual(["alias first", "alias second"]);
    expect(run(["--output-all", "raw"]).split("\n").map((line) => JSON.parse(line)).sort()).toEqual(["alias first", "alias second"]);

    run(["-c", "direct clipboard"]);
    expect(readFileSync(env.DITOX_CLIPBOARD_MOCK!, "utf8")).toBe("direct clipboard");
    expect(run(["-p"])).toBe("direct clipboard");

    expect(run(["favorite", "1"])).toBe("true");
    expect(run(["-a", "clear me"])).toBe("3");
    expect(run(["-clear"])).toBe("2");
    let status = JSON.parse(run(["status"]));
    expect(status.stats.entries).toBe(1);
    expect(status.stats.favorites).toBe(1);

    expect(run(["-a", "clear pinned too"])).toBe("4");
    expect(run(["-clear-text"])).toBe("2");
    status = JSON.parse(run(["status"]));
    expect(status.stats.entries).toBe(0);

    expect(run(["-a", "wipe one"])).toBe("5");
    expect(run(["-a", "wipe two"])).toBe("6");
    expect(run(["favorite", "5"])).toBe("true");
    expect(run(["-clear-all"])).toBe("2");
    status = JSON.parse(run(["status"]));
    expect(status.stats.entries).toBe(0);

    const clean = JSON.parse(run(["--clean"]));
    expect(clean.ok).toBe(true);
    expect(clean.sanitized_text).toBe(0);
    expect(clean.removed_missing_images).toBe(0);
  });

  test("recognizes unsupported platform-specific listener aliases", () => {
    expect(run(["-listen-x11"])).toContain("currently supports Wayland listening");
    expect(run(["--listen-darwin"])).toContain("currently supports Wayland listening");
  });

  test("stores wl-paste watch stdin through the wl-store alias", () => {
    expect(runWithStdin(["--wl-store"], "stored from wl watch")).toBe("");
    let listed = JSON.parse(run(["list"]));
    expect(listed.entries).toHaveLength(1);
    expect(listed.entries[0].kind).toBe("text");
    expect(listed.entries[0].content).toBe("stored from wl watch");

    const png = rgbaPng(3, 2, [
      [255, 0, 0, 255],
      [0, 255, 0, 255],
      [0, 0, 255, 255],
      [255, 255, 255, 255],
      [16, 32, 48, 255],
      [64, 80, 96, 255],
    ]);
    expect(runWithStdin(["--wl-store"], png)).toBe("");

    listed = JSON.parse(run(["list", "--filter", "images"]));
    expect(listed.entries).toHaveLength(1);
    expect(listed.entries[0].kind).toBe("image");
    expect(listed.entries[0].mime).toBe("image/png");
    expect(listed.entries[0].image_width).toBe(3);
    expect(listed.entries[0].image_height).toBe(2);
    expect(existsSync(listed.entries[0].blob_path)).toBe(true);

    expect(runWithStdin(["--wl-store"], "do not store", { CLIPBOARD_STATE: "sensitive" })).toBe("");
    const status = JSON.parse(run(["status"]));
    expect(status.stats.entries).toBe(2);
  });

  test("daemon ingest drops a sensitive clipboard offer before it reaches the socket", async () => {
    const runtime = join(tempDir, "run");
    const sock = join(runtime, "ditox", "rpc.sock");
    mkdirSync(join(runtime, "ditox"), { recursive: true });
    let connections = 0;
    const server = createServer((socket) => {
      connections += 1;
      socket.end();
    });
    await new Promise<void>((resolve, reject) => {
      server.once("error", reject);
      server.listen(sock, () => resolve());
    });
    const secret = "secret-from-watch";
    const launch = (state: string, body: string) =>
      Bun.spawn({
        cmd: [ditoxd, "ingest", "text"],
        env: { ...process.env, ...env, XDG_RUNTIME_DIR: runtime, CLIPBOARD_STATE: state },
        stdin: Buffer.from(body),
        stdout: "pipe",
        stderr: "pipe",
      });
    try {
      const blocked = launch("sensitive", secret);
      expect(await waitForExit(blocked, 2000)).toBe(0);
      expect(connections).toBe(0);
      expect(await new Response(blocked.stdout).text()).not.toContain(secret);
      expect(await new Response(blocked.stderr).text()).not.toContain(secret);

      const ordinary = launch("data", "ordinary-from-watch");
      expect(await waitForExit(ordinary, 2000)).toBe(0);
      expect(connections).toBe(1);
    } finally {
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  });

  test("verifies Hyprland paste-back and launch wiring with fake tools", () => {
    const fakeBin = join(tempDir, "bin");
    const fakeClipboard = join(tempDir, "fake-clipboard.txt");
    const hyprLog = join(tempDir, "hypr.log");
    const launchTarget = join(tempDir, "launch-target.txt");
    const keepTarget = join(tempDir, "keep-target.txt");
    const launchKeepTarget = join(tempDir, "launch-keep-target.txt");
    const noArgLaunchTarget = join(tempDir, "no-arg-launch-target.txt");
    const realtimeTarget = join(tempDir, "realtime-target.txt");
    const tuiRealtimeTarget = join(tempDir, "tui-realtime-target.txt");
    mkdirSync(fakeBin);

    writeFileSync(
      join(fakeBin, "wl-copy"),
      "#!/usr/bin/env sh\ncat > \"$DITOX_FAKE_CLIPBOARD\"\n",
    );
    writeFileSync(
      join(fakeBin, "hyprctl"),
      [
        "#!/usr/bin/env sh",
        "printf '%s\\n' \"$*\" >> \"$DITOX_HYPR_LOG\"",
        "if [ \"$1\" = \"-j\" ] && [ \"$2\" = \"activewindow\" ]; then",
        "  printf '{\"address\":\"0xabc;echo bad\",\"class\":\"fake-app\",\"title\":\"fake-title\"}'",
        "fi",
        "exit 0",
        "",
      ].join("\n"),
    );
    chmodSync(join(fakeBin, "wl-copy"), 0o755);
    chmodSync(join(fakeBin, "hyprctl"), 0o755);

    delete env.DITOX_CLIPBOARD_MOCK;
    env.PATH = `${fakeBin}:${process.env.PATH ?? ""}`;
    env.DITOX_FAKE_CLIPBOARD = fakeClipboard;
    env.DITOX_HYPR_LOG = hyprLog;

    expect(run(["add", "hypr paste target"])).toBe("1");
    run(["paste", "1", "--target-window", "0xabc"]);
    expect(readFileSync(fakeClipboard, "utf8")).toBe("hypr paste target");

    const log = readFileSync(hyprLog, "utf8");
    expect(log).toContain("dispatch focuswindow address:0xabc");
    expect(log).toContain("dispatch sendshortcut CTRL,V,");

    env.DITOX_TERMINAL_COMMAND = `sh -c 'printf "$DITOX_TARGET_WINDOW" > ${launchTarget}'`;
    run(["launch"]);
    expect(readFileSync(launchTarget, "utf8")).toBe("0xabc;echo bad");

    env.DITOX_TERMINAL_COMMAND = `sh -c 'printf "$DITOX_TARGET_WINDOW|$DITOX_TUI_EXIT_AFTER_PASTE" > ${keepTarget}'`;
    run(["keep"]);
    expect(readFileSync(keepTarget, "utf8")).toBe("0xabc;echo bad|false");

    env.DITOX_TERMINAL_COMMAND = `sh -c 'printf "$DITOX_TARGET_WINDOW|$DITOX_TUI_EXIT_AFTER_PASTE" > ${launchKeepTarget}'`;
    run(["launch", "--keep"]);
    expect(readFileSync(launchKeepTarget, "utf8")).toBe("0xabc;echo bad|false");

    env.DITOX_TERMINAL_COMMAND = `sh -c 'printf "$DITOX_TARGET_WINDOW" > ${noArgLaunchTarget}'`;
    run([]);
    expect(readFileSync(noArgLaunchTarget, "utf8")).toBe("0xabc;echo bad");

    env.DITOX_TERMINAL_COMMAND = `sh -c 'printf "$DITOX_TARGET_WINDOW|$DITOX_TUI_REFRESH_MS" > ${realtimeTarget}'`;
    run(["-enable-real-time"]);
    expect(readFileSync(realtimeTarget, "utf8")).toBe("0xabc;echo bad|250");

    env.DITOX_TUI_COMMAND = `sh -c 'printf "$DITOX_TARGET_WINDOW|$DITOX_TUI_REFRESH_MS" > ${tuiRealtimeTarget}'`;
    run(["tui", "--enable-real-time"]);
    expect(readFileSync(tuiRealtimeTarget, "utf8")).toBe("0xabc;echo bad|250");
    delete env.DITOX_TUI_COMMAND;

    env.DITOX_CONFIG = join(tempDir, "config.toml");
    writeFileSync(env.DITOX_CONFIG, "[auto_paste]\nenabled = true\nkeybind = \"ctrl+shift+v\"\nbuffer_ms = 0\n");
    run(["--auto-paste", "--target-window", "0xabc"]);
    const autoPasteLog = readFileSync(hyprLog, "utf8");
    expect(autoPasteLog).toContain("dispatch focuswindow address:0xabc");
    expect(autoPasteLog).toContain("dispatch sendshortcut CTRL SHIFT,V,");
    const status = JSON.parse(run(["status"]));
    expect(status.config.auto_paste_enabled).toBe(true);
    expect(status.config.auto_paste_keybind).toBe("ctrl+shift+v");
    expect(status.config.auto_paste_buffer_ms).toBe(0);
  });

  test("dispatches Hyprland 0.57 paste-back as Lua and keeps the old syntax before it", () => {
    expect(run(["add", "lua paste target"])).toBe("1");
    const { fakeBin, hyprLog, fakeClipboard, versionFile } = fakeHyprland("lua-bin");
    env.PATH = `${fakeBin}:${process.env.PATH ?? ""}`;

    writeFileSync(versionFile, HYPRLAND_0562_VERSION);
    run(["paste", "1", "--target-window", "0xabc"]);
    expect(hyprLines(hyprLog)).toEqual([
      "-j version",
      "dispatch focuswindow address:0xabc",
      "dispatch sendshortcut CTRL,V,",
    ]);

    writeFileSync(versionFile, HYPRLAND_0570_VERSION);
    writeFileSync(hyprLog, "");
    run(["paste", "1", "--target-window", "0xabc"]);
    expect(readFileSync(fakeClipboard, "utf8")).toBe("lua paste target");
    expect(hyprLines(hyprLog)).toEqual([
      "-j version",
      'dispatch hl.dsp.focus({ window = "address:0xabc" })',
      'dispatch hl.dsp.send_shortcut({ mods = "CTRL", key = "V" })',
    ]);

    env.DITOX_CONFIG = join(tempDir, "lua-config.toml");
    writeFileSync(env.DITOX_CONFIG, "[auto_paste]\nenabled = true\nkeybind = \"ctrl+shift+v\"\nbuffer_ms = 0\n");
    writeFileSync(hyprLog, "");
    run(["--auto-paste", "--target-window", "0xabc"]);
    expect(hyprLines(hyprLog)).toEqual([
      "-j version",
      'dispatch hl.dsp.focus({ window = "address:0xabc" })',
      'dispatch hl.dsp.send_shortcut({ mods = "CTRL SHIFT", key = "V" })',
    ]);

    // The target comes from the command line or the environment; it must
    // stay inside its Lua string.
    writeFileSync(hyprLog, "");
    run(["paste", "1", "--target-window", '0xabc" }) hl.dsp.exec_cmd("touch pwned") --\\']);
    expect(hyprLines(hyprLog)[1]).toBe(
      'dispatch hl.dsp.focus({ window = "address:0xabc\\" }) hl.dsp.exec_cmd(\\"touch pwned\\") --\\\\" })',
    );
  });

  test("treats a Hyprland 0.57 warning reply as a failed paste", () => {
    expect(run(["add", "warned paste"])).toBe("1");
    const { fakeBin, hyprLog, fakeClipboard, versionFile } = fakeHyprland("lua-fail-bin");
    env.PATH = `${fakeBin}:${process.env.PATH ?? ""}`;
    writeFileSync(versionFile, HYPRLAND_0570_VERSION);

    env.DITOX_FAKE_SHORTCUT_REPLY = "warning: no window to send the shortcut to";
    const warned = runResult(["paste", "1", "--target-window", "0xabc"]);
    expect(warned.exitCode).not.toBe(0);
    expect(warned.stderr).toContain("PasteBackFailed");
    expect(readFileSync(fakeClipboard, "utf8")).toBe("warned paste");

    env.DITOX_FAKE_SHORTCUT_REPLY = "error: 3 [string \"return hl.dispatch(bad)\"]:1: syntax error";
    env.DITOX_FAKE_SHORTCUT_EXIT = "7";
    const rejected = runResult(["paste", "1", "--target-window", "0xabc"]);
    expect(rejected.exitCode).not.toBe(0);
    expect(rejected.stderr).toContain("PasteBackFailed");

    // A target that Hyprland 0.57 cannot focus stops the paste before the
    // shortcut, so it never lands in whatever window has focus.
    delete env.DITOX_FAKE_SHORTCUT_REPLY;
    delete env.DITOX_FAKE_SHORTCUT_EXIT;
    env.DITOX_FAKE_FOCUS_REPLY = "warning: =[C]:-1: hl.focus: window not found";
    writeFileSync(hyprLog, "");
    const unfocused = runResult(["paste", "1", "--target-window", "0xabc"]);
    expect(unfocused.exitCode).not.toBe(0);
    expect(unfocused.stderr).toContain("PasteBackFailed");
    expect(hyprLines(hyprLog)).toEqual(["-j version", 'dispatch hl.dsp.focus({ window = "address:0xabc" })']);

    env.DITOX_FAKE_FOCUS_REPLY = "error: 3 [string \"return hl.dispatch(bad)\"]:1: syntax error";
    env.DITOX_FAKE_FOCUS_EXIT = "7";
    writeFileSync(hyprLog, "");
    const refused = runResult(["paste", "1", "--target-window", "0xabc"]);
    expect(refused.exitCode).not.toBe(0);
    expect(refused.stderr).toContain("PasteBackFailed");
    expect(hyprLines(hyprLog)).toEqual(["-j version", 'dispatch hl.dsp.focus({ window = "address:0xabc" })']);

    // Before 0.57 focus stays best effort, as it always was.
    delete env.DITOX_FAKE_FOCUS_REPLY;
    delete env.DITOX_FAKE_FOCUS_EXIT;
    env.DITOX_FAKE_LEGACY_FOCUS_EXIT = "1";
    writeFileSync(versionFile, HYPRLAND_0562_VERSION);
    writeFileSync(hyprLog, "");
    run(["paste", "1", "--target-window", "0xabc"]);
    expect(hyprLines(hyprLog)).toEqual([
      "-j version",
      "dispatch focuswindow address:0xabc",
      "dispatch sendshortcut CTRL,V,",
    ]);
  });

  test("retries in Lua when the version is unknown but Hyprland parses Lua", () => {
    expect(run(["add", "retried paste"])).toBe("1");
    const { fakeBin, hyprLog, versionFile } = fakeHyprland("lua-retry-bin");
    env.PATH = `${fakeBin}:${process.env.PATH ?? ""}`;
    env.DITOX_FAKE_LUA_ONLY = "1";
    writeFileSync(versionFile, "");

    run(["paste", "1", "--target-window", "0xabc"]);
    expect(hyprLines(hyprLog)).toEqual([
      "-j version",
      "dispatch focuswindow address:0xabc",
      'dispatch hl.dsp.focus({ window = "address:0xabc" })',
      'dispatch hl.dsp.send_shortcut({ mods = "CTRL", key = "V" })',
    ]);

    // After the switch to Lua the focus reply counts like any other 0.57 one.
    env.DITOX_FAKE_FOCUS_REPLY = "warning: =[C]:-1: hl.focus: window not found";
    writeFileSync(hyprLog, "");
    const unfocused = runResult(["paste", "1", "--target-window", "0xabc"]);
    expect(unfocused.exitCode).not.toBe(0);
    expect(unfocused.stderr).toContain("PasteBackFailed");
    expect(hyprLines(hyprLog)).toEqual([
      "-j version",
      "dispatch focuswindow address:0xabc",
      'dispatch hl.dsp.focus({ window = "address:0xabc" })',
    ]);
    delete env.DITOX_FAKE_FOCUS_REPLY;

    env.DITOX_CONFIG = join(tempDir, "retry-config.toml");
    writeFileSync(env.DITOX_CONFIG, "[auto_paste]\nenabled = true\nbuffer_ms = 0\n");
    writeFileSync(hyprLog, "");
    run(["--auto-paste"]);
    expect(hyprLines(hyprLog)).toEqual([
      "-j version",
      "dispatch sendshortcut CTRL,V,",
      'dispatch hl.dsp.send_shortcut({ mods = "CTRL", key = "V" })',
    ]);
  });

  test("does not wait for a wl-copy clipboard owner that keeps stderr open", () => {
    expect(run(["add", "clipboard owner text"])).toBe("1");

    const fakeBin = join(tempDir, "owner-bin");
    const fakeClipboard = join(tempDir, "owner-clipboard.txt");
    const ownerPidFile = join(tempDir, "owner.pid");
    mkdirSync(fakeBin);
    writeFileSync(
      join(fakeBin, "wl-copy"),
      [
        "#!/usr/bin/env sh",
        'cat > "$DITOX_FAKE_CLIPBOARD"',
        "(sh -c 'while :; do sleep 60; done' >&2) &",
        'printf "%s\\n" "$!" > "$DITOX_WL_OWNER_PID_FILE"',
        "exit 0",
        "",
      ].join("\n"),
    );
    chmodSync(join(fakeBin, "wl-copy"), 0o755);

    delete env.DITOX_CLIPBOARD_MOCK;
    env.PATH = `${fakeBin}:${process.env.PATH ?? ""}`;
    env.DITOX_FAKE_CLIPBOARD = fakeClipboard;
    env.DITOX_WL_OWNER_PID_FILE = ownerPidFile;

    const proc = Bun.spawnSync({
      cmd: ["timeout", "--foreground", "--kill-after=1s", "2s", ditox, "copy", "1"],
      env: { ...process.env, ...env },
      stdout: "pipe",
      stderr: "pipe",
    });
    try {
      expect(proc.exitCode).toBe(0);
      expect(readFileSync(fakeClipboard, "utf8")).toBe("clipboard owner text");
    } finally {
      if (existsSync(ownerPidFile)) {
        const pid = Number(readFileSync(ownerPidFile, "utf8").trim());
        if (Number.isFinite(pid) && pid > 0) {
          try {
            process.kill(pid, "SIGTERM");
          } catch {}
        }
      }
    }
  });

  test("runs bundled TUI from an install layout outside the repo checkout", () => {
    expect(existsSync(join(repoRoot, "zig-out", "share", "ditox", "tui", "dist", "index.js"))).toBe(true);
    expect(existsSync(join(repoRoot, "zig-out", "share", "ditox", "tui", "tui-config.schema.json"))).toBe(true);
    expect(existsSync(join(repoRoot, "zig-out", "share", "ditox", "tui", "tui-config.example.json"))).toBe(true);

    const installRoot = join(tempDir, "install");
    const installBin = join(installRoot, "bin");
    const tuiDist = join(installRoot, "share", "ditox", "tui", "dist");
    const fakeBin = join(tempDir, "fake-bin");
    const bunArgs = join(tempDir, "bun-args.txt");
    const bunEnv = join(tempDir, "bun-env.txt");
    mkdirSync(installBin, { recursive: true });
    mkdirSync(tuiDist, { recursive: true });
    mkdirSync(fakeBin, { recursive: true });

    const installedDitox = join(installBin, "ditox");
    copyFileSync(ditox, installedDitox);
    chmodSync(installedDitox, 0o755);
    writeFileSync(join(tuiDist, "index.js"), "console.log('bundled tui placeholder');\n");
    writeFileSync(
      join(fakeBin, "bun"),
      [
        "#!/usr/bin/env sh",
        "printf '%s\\n' \"$*\" > \"$DITOX_BUN_ARGS\"",
        "printf '%s|%s\\n' \"$DITOX_TARGET_WINDOW\" \"$DITOX_TUI_REFRESH_MS\" > \"$DITOX_BUN_ENV\"",
        "",
      ].join("\n"),
    );
    writeFileSync(
      join(fakeBin, "hyprctl"),
      [
        "#!/usr/bin/env sh",
        "if [ \"$1\" = \"-j\" ] && [ \"$2\" = \"activewindow\" ]; then",
        "  printf '{\"address\":\"0xinstalled\",\"class\":\"fake-app\",\"title\":\"fake-title\"}'",
        "fi",
        "",
      ].join("\n"),
    );
    chmodSync(join(fakeBin, "bun"), 0o755);
    chmodSync(join(fakeBin, "hyprctl"), 0o755);

    const proc = Bun.spawnSync({
      cmd: [installedDitox, "tui", "--enable-real-time"],
      env: {
        ...process.env,
        ...env,
        PATH: `${fakeBin}:${process.env.PATH ?? ""}`,
        DITOX_BUN_ARGS: bunArgs,
        DITOX_BUN_ENV: bunEnv,
      },
      stdout: "pipe",
      stderr: "pipe",
    });

    expect(proc.exitCode).toBe(0);
    const capturedBun = readFileSync(bunArgs, "utf8").trim();
    // The install-layout launch must now use the hermetic form so the TUI
    // never resolves @opentui from the user's global Bun cache (the root
    // cause of the Super+V crash with "dumpStdoutBuffer not found").
    expect(capturedBun).toContain("--no-install");
    expect(capturedBun).toContain("--cwd");
    // After --cwd the command uses the relative ./dist/index.js (correct hermetic form)
    expect(capturedBun).toContain("dist/index.js");
    expect(readFileSync(bunEnv, "utf8").trim()).toBe("0xinstalled|250");
  });

  test("kills the stored watcher process through the kill alias", async () => {
    env.DITOX_CONFIG = join(tempDir, "watcher.toml");
    writeFileSync(env.DITOX_CONFIG, "[watch]\npoll_interval_ms = 50\n");

    const fakeBin = join(tempDir, "watcher-bin");
    mkdirSync(fakeBin);
    writeFileSync(join(fakeBin, "wl-paste"), "#!/usr/bin/env sh\nexit 1\n");
    writeFileSync(join(fakeBin, "hyprctl"), "#!/usr/bin/env sh\nexit 1\n");
    chmodSync(join(fakeBin, "wl-paste"), 0o755);
    chmodSync(join(fakeBin, "hyprctl"), 0o755);
    env.PATH = `${fakeBin}:${process.env.PATH ?? ""}`;

    const proc = Bun.spawn({
      cmd: [ditoxd, "watch"],
      env: { ...process.env, ...env },
      stdout: "pipe",
      stderr: "pipe",
    });

    try {
      await waitUntil(() => JSON.parse(run(["status"])).watcher.running === true, 2500);
      const result = JSON.parse(run(["--kill"]));
      expect(result.killed).toBe(true);
      expect(typeof result.pid).toBe("number");
      expect(await waitForExit(proc, 2500)).not.toBeNull();
      const stale = JSON.parse(run(["--kill"]));
      expect(stale.killed).toBe(false);
    } finally {
      proc.kill();
      await proc.exited.catch(() => {});
    }
  });

  test("skips excluded active apps and windows case-insensitively while watching", async () => {
    env.DITOX_CONFIG = join(tempDir, "excluded.toml");
    writeFileSync(
      env.DITOX_CONFIG,
      [
        'excludedApps = ["Bitwarden"]',
        'excludedWindows = ["Secret Window"]',
        "",
        "[watch]",
        "poll_interval_ms = 50",
        "",
      ].join("\n"),
    );

    const fakeBin = join(tempDir, "excluded-bin");
    mkdirSync(fakeBin);
    writeFileSync(
      join(fakeBin, "wl-paste"),
      [
        "#!/usr/bin/env sh",
        'if [ "$1" = "--list-types" ]; then',
        "  printf 'text/plain\\n'",
        "  exit 0",
        "fi",
        "printf 'excluded clipboard text'",
        "",
      ].join("\n"),
    );
    writeFileSync(
      join(fakeBin, "hyprctl"),
      [
        "#!/usr/bin/env sh",
        'if [ "$1" = "-j" ] && [ "$2" = "activewindow" ]; then',
        '  printf \'{"address":"0xabc","class":"bitwarden","title":"secret window"}\'',
        "fi",
        "exit 0",
        "",
      ].join("\n"),
    );
    chmodSync(join(fakeBin, "wl-paste"), 0o755);
    chmodSync(join(fakeBin, "hyprctl"), 0o755);
    delete env.DITOX_CLIPBOARD_MOCK;
    env.PATH = `${fakeBin}:${process.env.PATH ?? ""}`;

    const proc = Bun.spawn({
      cmd: [ditoxd, "watch"],
      env: { ...process.env, ...env },
      stdout: "pipe",
      stderr: "pipe",
    });

    try {
      await waitUntil(() => JSON.parse(run(["status"])).watcher.running === true, 2500);
      await sleep(180);
      const status = JSON.parse(run(["status"]));
      expect(status.config.excluded_apps).toEqual(["Bitwarden"]);
      expect(status.config.excluded_windows).toEqual(["Secret Window"]);
      expect(status.stats.entries).toBe(0);
      expect(JSON.parse(run(["list"])).entries).toHaveLength(0);
    } finally {
      proc.kill();
      await proc.exited.catch(() => {});
    }
  });

  test("copies and pastes image entries as image bytes with their MIME type", () => {
    JSON.parse(run(["status"]));

    const imageDir = join(env.DITOX_DATA_DIR!, "images-v2", "12");
    mkdirSync(imageDir, { recursive: true });
    const imagePath = join(imageDir, "123456.png");
    const png = rgbaPng(1, 1, [[16, 32, 48, 255]]);
    writeFileSync(imagePath, png);

    const db = new Database(join(env.DITOX_DATA_DIR!, "ditox-v2.db"));
    db.run(
      "INSERT INTO entries (kind, mime, content, preview, hash, favorite, created_at_ms, byte_len, blob_path, image_width, image_height) VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?)",
      "image",
      "image/png",
      "123456",
      "image/png 1x1",
      "123456",
      Date.now(),
      png.length,
      imagePath,
      1,
      1,
    );
    db.close();

    const fakeBin = join(tempDir, "bin");
    const fakeClipboard = join(tempDir, "fake-image-clipboard.bin");
    const wlCopyArgs = join(tempDir, "wl-copy.args");
    const hyprLog = join(tempDir, "hypr-image.log");
    mkdirSync(fakeBin);
    writeFileSync(
      join(fakeBin, "wl-copy"),
      "#!/usr/bin/env sh\nprintf '%s\\n' \"$*\" > \"$DITOX_WL_COPY_ARGS\"\ncat > \"$DITOX_FAKE_CLIPBOARD\"\n",
    );
    writeFileSync(
      join(fakeBin, "hyprctl"),
      "#!/usr/bin/env sh\nprintf '%s\\n' \"$*\" >> \"$DITOX_HYPR_LOG\"\nexit 0\n",
    );
    chmodSync(join(fakeBin, "wl-copy"), 0o755);
    chmodSync(join(fakeBin, "hyprctl"), 0o755);

    delete env.DITOX_CLIPBOARD_MOCK;
    env.PATH = `${fakeBin}:${process.env.PATH ?? ""}`;
    env.DITOX_FAKE_CLIPBOARD = fakeClipboard;
    env.DITOX_WL_COPY_ARGS = wlCopyArgs;
    env.DITOX_HYPR_LOG = hyprLog;

    run(["copy", "1"]);
    expect(readFileSync(wlCopyArgs, "utf8").trim()).toBe("--type image/png");
    expect([...readFileSync(fakeClipboard)]).toEqual([...png]);

    run(["paste", "1", "--target-window", "0xdef"]);
    expect(readFileSync(wlCopyArgs, "utf8").trim()).toBe("--type image/png");
    expect([...readFileSync(fakeClipboard)]).toEqual([...png]);
    const log = readFileSync(hyprLog, "utf8");
    expect(log).toContain("dispatch focuswindow address:0xdef");
    expect(log).toContain("dispatch sendshortcut CTRL,V,");
  });

  test("renders PNG image history as terminal block cells in the TUI smoke path", () => {
    expect(run(["add", "text seed"])).toBe("1");

    const imageDir = join(env.DITOX_DATA_DIR!, "images-v2", "ab");
    mkdirSync(imageDir, { recursive: true });
    const imagePath = join(imageDir, "abcdef.png");
    const png = rgbaPng(2, 2, [
      [255, 0, 0, 255],
      [0, 255, 0, 255],
      [0, 0, 255, 255],
      [255, 255, 255, 255],
    ]);
    writeFileSync(imagePath, png);

    const db = new Database(join(env.DITOX_DATA_DIR!, "ditox-v2.db"));
    db.run(
      "INSERT INTO entries (kind, mime, content, preview, hash, favorite, created_at_ms, byte_len, blob_path, image_width, image_height) VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?)",
      "image",
      "image/png",
      "abcdef",
      "image/png 2x2",
      "abcdef",
      Date.now() + 1000,
      png.length,
      imagePath,
      2,
      2,
    );
    db.close();

    // Image previews decode asynchronously (bytes arrive over RPC); the
    // renderer only emits the post-decode frame on a real terminal, so run
    // the TUI under a PTY via `script` instead of bare pipes.
    const proc = Bun.spawnSync({
      cmd: ["script", "-qec", "timeout --foreground --kill-after=1s 1s bun run --cwd tui start", "/dev/null"],
      cwd: repoRoot,
      env: {
        ...process.env,
        ...env,
        DITOXD: ditoxd,
        DITOX_TUI_REFRESH_MS: "0",
        DITOX_TUI_IMAGE_PREVIEW: "blocks",
      },
      stdout: "pipe",
      stderr: "pipe",
    });

    expect(proc.exitCode).toBe(124);
    expect(`${proc.stdout?.toString() ?? ""}${proc.stderr?.toString() ?? ""}`).toContain("▀");
  });

  test("renders file-based TUI customization in the real OpenTUI smoke path", () => {
    expect(run(["add", "visual smoke entry"])).toBe("1");
    const tuiConfigPath = join(tempDir, "tui.json");
    writeFileSync(
      tuiConfigPath,
      JSON.stringify({
        layout: {
          refreshIntervalMs: 0,
          listWidthPercent: 50,
          showMetadata: true,
          imagePreviewMode: "metadata",
          mouseEnabled: false,
        },
        chrome: {
          panelBorderStyle: "double",
          selectedMarker: "!!",
          normalMarker: "..",
          statusSeparator: "::",
        },
        labels: {
          appTitle: "visual",
          brand: "VIS",
          historyTitle: "clips",
          previewTitle: "peek",
          watcherStopped: "daemon off",
          pinned: "SAVED",
        },
        keyBindings: {
          copyPaste: "ctrl+p",
          preview: "ctrl+o",
          help: "f1",
        },
      }),
    );

    const proc = Bun.spawnSync({
      cmd: ["timeout", "--foreground", "--kill-after=1s", "1s", "bun", "run", "--cwd", "tui", "start"],
      cwd: repoRoot,
      env: {
        ...process.env,
        ...env,
        DITOXD: ditoxd,
        DITOX_TUI_CONFIG: tuiConfigPath,
      },
      stdout: "pipe",
      stderr: "pipe",
    });

    expect(proc.exitCode).toBe(124);
    const output = `${proc.stdout?.toString() ?? ""}${proc.stderr?.toString() ?? ""}`;
    expect(output).toContain("VIS");
    expect(output).toContain("clips");
    expect(output).toContain("peek");
    expect(output).toContain("visual smoke entry");
    expect(output).toContain("!!");
    expect(output).toContain("ctrl+p paste");
    expect(output).toContain("ctrl+o preview");
    expect(output).toContain("::");
    expect(output).toContain("daemon off");
  });

  test("expires non-pinned entries by configured TTL and removes image blobs", () => {
    env.DITOX_CONFIG = join(tempDir, "config.toml");
    writeFileSync(env.DITOX_CONFIG, "[history]\ndelete_after_seconds = 1\n");

    expect(run(["add", "old unpinned"])).toBe("1");
    expect(run(["add", "old pinned"])).toBe("2");
    expect(run(["favorite", "2"])).toBe("true");

    const imageDir = join(env.DITOX_DATA_DIR!, "images-v2", "de");
    mkdirSync(imageDir, { recursive: true });
    const imagePath = join(imageDir, "deadbeef.png");
    writeFileSync(imagePath, rgbaPng(1, 1, [[255, 0, 0, 255]]));

    const db = new Database(join(env.DITOX_DATA_DIR!, "ditox-v2.db"));
    db.run(
      "INSERT INTO entries (kind, mime, content, preview, hash, favorite, created_at_ms, byte_len, blob_path, image_width, image_height) VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?)",
      "image",
      "image/png",
      "deadbeef",
      "image/png 1x1",
      "deadbeef",
      Date.now() - 10_000,
      70,
      imagePath,
      1,
      1,
    );
    db.run("UPDATE entries SET created_at_ms = ? WHERE id IN (1, 2)", Date.now() - 10_000);
    db.close();

    const status = JSON.parse(run(["status"]));
    expect(status.config.delete_after_seconds).toBe(1);
    expect(status.stats.entries).toBe(1);
    expect(status.stats.favorites).toBe(1);
    expect(existsSync(imagePath)).toBe(false);

    const listed = JSON.parse(run(["list"]));
    expect(listed.entries).toHaveLength(1);
    expect(listed.entries[0].content).toBe("old pinned");
  });
});

function run(args: string[]): string {
  const proc = Bun.spawnSync({
    cmd: [ditox, ...args],
    env: { ...process.env, ...env },
    stdout: "pipe",
    stderr: "pipe",
  });

  if (proc.exitCode !== 0) {
    throw new Error(`${args.join(" ")} failed: ${proc.stderr?.toString() ?? ""}`);
  }
  return (proc.stdout?.toString() ?? "").trim();
}

function runWithStdin(args: string[], stdin: string | Uint8Array, extraEnv: Record<string, string> = {}): string {
  const proc = Bun.spawnSync({
    cmd: [ditox, ...args],
    env: { ...process.env, ...env, ...extraEnv },
    stdin: typeof stdin === "string" ? Buffer.from(stdin) : Buffer.from(stdin),
    stdout: "pipe",
    stderr: "pipe",
  });

  if (proc.exitCode !== 0) {
    throw new Error(`${args.join(" ")} failed: ${proc.stderr?.toString() ?? ""}`);
  }
  return (proc.stdout?.toString() ?? "").trim();
}

async function waitUntil(predicate: () => boolean, timeoutMs: number): Promise<void> {
  const started = Date.now();
  let lastError: unknown;
  while (Date.now() - started < timeoutMs) {
    try {
      if (predicate()) return;
    } catch (error) {
      lastError = error;
    }
    await sleep(50);
  }
  if (lastError) throw lastError;
  throw new Error(`timed out after ${timeoutMs}ms`);
}

async function waitForExit(proc: Bun.Subprocess, timeoutMs: number): Promise<number | null> {
  return await Promise.race([proc.exited, sleep(timeoutMs).then(() => null)]);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function rgbaPng(width: number, height: number, pixels: Array<[number, number, number, number]>): Uint8Array {
  const raw = new Uint8Array(height * (1 + width * 4));
  let offset = 0;
  let pixel = 0;
  for (let y = 0; y < height; y += 1) {
    raw[offset++] = 0;
    for (let x = 0; x < width; x += 1) {
      raw.set(pixels[pixel++]!, offset);
      offset += 4;
    }
  }

  return concatBytes(
    Uint8Array.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", concatBytes(u32(width), u32(height), Uint8Array.from([8, 6, 0, 0, 0]))),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", new Uint8Array()),
  );
}

function chunk(type: string, data: Uint8Array): Uint8Array {
  const typeBytes = Uint8Array.from([...type].map((char) => char.charCodeAt(0)));
  return concatBytes(u32(data.length), typeBytes, data, Uint8Array.from([0, 0, 0, 0]));
}

function u32(value: number): Uint8Array {
  return Uint8Array.from([(value >>> 24) & 0xff, (value >>> 16) & 0xff, (value >>> 8) & 0xff, value & 0xff]);
}

function concatBytes(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

// Trimmed from real `hyprctl -j version` replies.
const HYPRLAND_0562_VERSION = JSON.stringify({
  branch: "v0.56.2-b",
  commit: "efb50993780079460b0cbed1363e2166a2de1d9f",
  version: "0.56.2",
  dirty: false,
  tag: "v0.56.2",
  flags: ["nix"],
});
const HYPRLAND_0570_VERSION = JSON.stringify({
  branch: "main",
  commit: "0aa6745f705f2c4b0a5004eb019ef76e5bbdb426",
  version: "0.57.0",
  dirty: false,
  tag: "v0.57.0",
  flags: ["nix"],
});

/**
 * A fake `hyprctl` that logs its argv, answers `-j version` from a file and
 * replies to dispatches like Hyprland 0.57 (`ok`, or the reply/exit status in
 * the DITOX_FAKE_* variables). With DITOX_FAKE_LUA_ONLY a legacy dispatch gets
 * the Lua parse error and exit status 7, as on a real 0.57;
 * DITOX_FAKE_LEGACY_FOCUS_EXIT makes a legacy `focuswindow` fail.
 */
function fakeHyprland(name: string): { fakeBin: string; hyprLog: string; fakeClipboard: string; versionFile: string } {
  const fakeBin = join(tempDir, name);
  const hyprLog = join(tempDir, `${name}.log`);
  const fakeClipboard = join(tempDir, `${name}-clipboard.txt`);
  const versionFile = join(tempDir, `${name}-version.json`);
  mkdirSync(fakeBin);
  writeFileSync(join(fakeBin, "wl-copy"), "#!/usr/bin/env sh\ncat > \"$DITOX_FAKE_CLIPBOARD\"\n");
  writeFileSync(
    join(fakeBin, "hyprctl"),
    [
      "#!/usr/bin/env sh",
      "printf '%s\\n' \"$*\" >> \"$DITOX_HYPR_LOG\"",
      "if [ \"$1\" = \"-j\" ] && [ \"$2\" = \"version\" ]; then",
      "  cat \"$DITOX_FAKE_HYPR_VERSION\"",
      "  exit 0",
      "fi",
      "if [ \"$1\" = \"dispatch\" ]; then",
      "  case \"$2\" in",
      "    hl.dsp.focus*) printf '%s\\n' \"${DITOX_FAKE_FOCUS_REPLY:-ok}\"; exit \"${DITOX_FAKE_FOCUS_EXIT:-0}\" ;;",
      "    hl.dsp.send_shortcut*) printf '%s\\n' \"${DITOX_FAKE_SHORTCUT_REPLY:-ok}\"; exit \"${DITOX_FAKE_SHORTCUT_EXIT:-0}\" ;;",
      "  esac",
      "  if [ -n \"$DITOX_FAKE_LUA_ONLY\" ]; then",
      "    printf 'error: 3 [string \"return hl.dispatch(%s %s)\"]:1: syntax error\\n' \"$2\" \"$3\"",
      "    exit 7",
      "  fi",
      "  if [ \"$2\" = \"focuswindow\" ] && [ -n \"$DITOX_FAKE_LEGACY_FOCUS_EXIT\" ]; then",
      "    printf 'No such window found\\n'",
      "    exit \"$DITOX_FAKE_LEGACY_FOCUS_EXIT\"",
      "  fi",
      "  printf 'ok\\n'",
      "fi",
      "exit 0",
      "",
    ].join("\n"),
  );
  chmodSync(join(fakeBin, "wl-copy"), 0o755);
  chmodSync(join(fakeBin, "hyprctl"), 0o755);
  writeFileSync(hyprLog, "");
  delete env.DITOX_CLIPBOARD_MOCK;
  env.DITOX_FAKE_CLIPBOARD = fakeClipboard;
  env.DITOX_HYPR_LOG = hyprLog;
  env.DITOX_FAKE_HYPR_VERSION = versionFile;
  return { fakeBin, hyprLog, fakeClipboard, versionFile };
}

function hyprLines(path: string): string[] {
  return readFileSync(path, "utf8").split("\n").filter((line) => line.length > 0);
}

function runResult(args: string[]): { exitCode: number | null; stdout: string; stderr: string } {
  const proc = Bun.spawnSync({
    cmd: [ditox, ...args],
    env: { ...process.env, ...env },
    stdout: "pipe",
    stderr: "pipe",
  });
  return { exitCode: proc.exitCode, stdout: proc.stdout?.toString() ?? "", stderr: proc.stderr?.toString() ?? "" };
}
