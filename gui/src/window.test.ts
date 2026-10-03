import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

// The window chrome lives in tauri.conf.json and picker.css, which no component test renders. These read the two files
// so a regression in the numbers and rules that fixed a visible bug fails here; the behaviour itself was checked in a
// browser (see the commit messages).
const css = readFileSync(new URL("./picker.css", import.meta.url), "utf8");
const conf = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8")) as {
  app: { windows: { label: string; width: number; height: number }[] };
};

function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(css);
  if (!match) throw new Error(`no rule for ${selector} in picker.css`);
  return match[1]!;
}

test("the options window is tall enough for its content, and the content scrolls inside it when it is not", () => {
  const options = conf.app.windows.find((window) => window.label === "options");
  // The content measures 342 px (IBM Plex Sans Condensed, two fieldsets and OK); 320 px clipped the OK button.
  expect(options?.height).toBeGreaterThanOrEqual(380);
  const box = rule(".ditox-options");
  expect(box).toMatch(/overflow(-y)?:\s*auto/);
  expect(box).toMatch(/(^|[\s;])height:\s*100%/);
  // min-height would let the box outgrow the window so the document, not the box, scrolled and OK sat under the edge.
  expect(box).not.toMatch(/min-height/);
});

test("a row aligns its text to the start of the line, not to the left, so right-to-left previews align right", () => {
  const row = rule(".ditox-row");
  expect(row).toMatch(/text-align:\s*start\s*;/);
  expect(row).not.toMatch(/text-align:\s*(left|right)/);
});
