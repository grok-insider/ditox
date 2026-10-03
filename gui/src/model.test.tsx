import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { PickerView } from "./Picker";
import {
  SELF_ACTIVATING_TARGETS,
  activatesItself,
  deliveryRequest,
  entriesFromList,
  filterEntries,
  imagePreviewSrc,
  moveSelection,
  pasteRequest,
  pasteSelection,
  pickerModel,
  listRequest,
  previewLines,
  refreshPicker,
  selectionAfterKey,
  shownImage,
  transparencyAlpha,
  visiblePreview,
  type PickerEntry,
} from "./model";

const rows: PickerEntry[] = [
  { id: 1, kind: "text", mime: "text/plain", preview: "hello", content: "hello clipboard" },
  { id: 2, kind: "image", mime: "image/png", preview: "https://example.com/clip", content: "hash" },
  { id: 3, kind: "text", mime: "text/html", preview: "other", content: "<p>not html rendered</p>" },
];

test("a missing socket is daemon-down and a connected empty list is empty", async () => {
  expect(pickerModel(false, rows)).toEqual({ status: "daemon-down" });
  expect(pickerModel(true, [])).toEqual({ status: "empty" });
  const down = await refreshPicker(
    {
      status: async () => "daemon-down",
      list: async () => {
        throw new Error("list");
      },
    },
    pickerModel(false, []),
  );
  expect(down).toEqual({ status: "daemon-down" });
  const empty = await refreshPicker(
    { status: async () => "connect", list: async () => ({ result: { entries: [] } }) },
    { status: "empty" },
  );
  expect(empty).toEqual({ status: "empty" });
  expect(renderToStaticMarkup(<PickerView model={down} query="" onQuery={() => undefined} onPaste={() => undefined} />)).toContain(
    "The daemon is down.",
  );
  expect(
    renderToStaticMarkup(<PickerView model={empty} query="" onQuery={() => undefined} onPaste={() => undefined} />),
  ).toContain("Empty history.");
});

test("search filters the loaded rows", () => {
  expect(filterEntries(rows, "example.com").map((entry) => entry.id)).toEqual([2]);
  expect(filterEntries(rows, "   ").map((entry) => entry.id)).toEqual([1, 2, 3]);
  expect(listRequest("  example.com ")).toEqual({ query: "example.com", limit: 1000 });
  expect(pickerModel(true, [], 0, "example.com")).toEqual({ status: "no-results", query: "example.com" });
  expect(previewLines(1)).toBe(1);
  expect(previewLines(4)).toBe(2);
  const html = renderToStaticMarkup(
    <PickerView model={pickerModel(true, rows)} query="" onQuery={() => undefined} onPaste={() => undefined} />,
  );
  expect(html.indexOf("hello clipboard")).toBeLessThan(html.indexOf('aria-label="Search"'));
  expect(html).toContain("…");
  expect(html).not.toContain(">Paste<");
  expect(html).not.toContain(">Plain<");
  expect(html).not.toContain("History");
  const missed = renderToStaticMarkup(
    <PickerView model={pickerModel(true, [], 0, "missing")} query="missing" onQuery={() => undefined} onPaste={() => undefined} />,
  );
  expect(missed).toContain("No results.");
  expect(missed.indexOf("No results.")).toBeLessThan(missed.indexOf('aria-label="Search"'));
});

test("a search asks the daemon and an empty hit is no results", async () => {
  let asked = "";
  const model = await refreshPicker(
    {
      status: async () => "connect",
      list: async (query) => {
        asked = query;
        return { result: { entries: [] } };
      },
    },
    { status: "empty" },
    "  clip ",
  );
  expect(asked).toBe("clip");
  expect(model).toEqual({ status: "no-results", query: "clip" });
});

test("transparency keeps the readable steps", () => {
  expect(transparencyAlpha(0)).toBe(1);
  expect(transparencyAlpha(10)).toBe(0.9);
  expect(transparencyAlpha(20)).toBe(0.8);
  expect(transparencyAlpha(30)).toBe(0.7);
  expect(transparencyAlpha(15)).toBe(1);
  expect(transparencyAlpha(40)).toBe(1);
});

test("original and plain paste are distinct requests", async () => {
  expect(pasteRequest(2, "original", "0xabc")).toEqual({
    method: "entries.paste",
    params: { id: 2, mode: "original", target_window: "0xabc" },
  });
  expect(pasteRequest(2, "plain", "0xabc").params.mode).toBe("plain");
  expect(deliveryRequest(4, "plain", null)).toEqual({ method: "entries.paste", params: { id: 4, mode: "plain" } });
  expect(deliveryRequest(4, "original", "  ").params).toEqual({ id: 4, mode: "original" });
  const sent: unknown[] = [];
  let hidden = false;
  await pasteSelection(
    {
      target: async () => "0xabc",
      paste: async (request) => {
        sent.push(request);
      },
      hide: async () => {
        hidden = true;
      },
    },
    2,
    "plain",
  );
  expect(sent).toEqual([pasteRequest(2, "plain", "0xabc")]);
  expect(hidden).toBe(true);
  expect(selectionAfterKey("Enter", false, false)).toEqual({ kind: "paste", mode: "original" });
  expect(selectionAfterKey("v", true, true)).toEqual({ kind: "paste", mode: "plain" });
});

test("preview is escaped text plus the selected image and blob paths stay out", () => {
  const blobPath = "/home/friend/.local/share/ditox/images-v2/ab/secret.png";
  const entries = entriesFromList({
    result: {
      entries: [
        { id: 9, kind: "image", mime: "image/png", preview: "https://example.com/a", content: "abc", blob_path: blobPath },
        { id: 10, kind: "text", mime: "text/html", preview: "x", content: "<p>not html rendered</p>", blob_path: blobPath },
      ],
    },
  });
  expect(JSON.stringify(entries)).not.toContain("blob_path");
  expect(JSON.stringify(entries)).not.toContain(blobPath);
  expect(visiblePreview(entries[1]!)).toBe("<p>not html rendered</p>");
  expect(imagePreviewSrc("image/png", "/w==")).toBe("data:image/png;base64,/w==");
  expect(shownImage("data:image/png;base64,/w==")).toBe("data:image/png;base64,/w==");
  expect(imagePreviewSrc("image/png", blobPath)).toBeNull();
  expect(shownImage(blobPath)).toBeNull();
  const html = renderToStaticMarkup(
    <PickerView
      model={pickerModel(true, entries, 0)}
      query=""
      imageSrc={imagePreviewSrc("image/png", "/w==")}
      onQuery={() => undefined}
      onPaste={() => undefined}
    />,
  );
  expect(html).toContain("&lt;p&gt;not html rendered&lt;/p&gt;");
  expect(html).not.toContain("<p>not html rendered</p>");
  expect(html).not.toContain(blobPath);
  expect(html).toContain("data:image/png;base64,/w==");
  const ready = pickerModel(true, rows);
  expect(moveSelection(ready, -1)).toMatchObject({ selected: 0 });
  expect(moveSelection(ready, 5)).toMatchObject({ selected: 2 });
});

// bun has no DOM. A tiny evaluator for exactly the selector forms SELF_ACTIVATING_TARGETS may use, run over fake nodes,
// so these tests exercise the real exported selector string (a fake `closest` that ignored its argument proves nothing)
// and fail loudly if the selector grows a form this evaluator does not understand.
type FakeNode = { class?: string[]; role?: string; parent?: FakeNode };

function matchesSimple(node: FakeNode, simple: string): boolean {
  const byClass = /^\.([\w-]+)$/.exec(simple);
  if (byClass) return node.class?.includes(byClass[1]!) ?? false;
  const rolePrefix = /^\[role\^="([\w-]+)"\]$/.exec(simple);
  if (rolePrefix) return node.role?.startsWith(rolePrefix[1]!) ?? false;
  const roleExact = /^\[role="([\w-]+)"\]$/.exec(simple);
  if (roleExact) return node.role === roleExact[1];
  throw new Error(`the test selector evaluator does not understand "${simple}"`);
}

function fakeTarget(node: FakeNode): EventTarget {
  return {
    closest(selector: string) {
      const parts = selector.split(",").map((part) => part.trim());
      for (let current: FakeNode | undefined = node; current; current = current.parent) {
        if (parts.some((part) => matchesSimple(current!, part))) return current;
      }
      return null;
    },
  } as unknown as EventTarget;
}

const more: FakeNode = { class: ["ditox-more"] };
const item: FakeNode = { role: "menuitem", parent: { role: "menu", class: ["ditox-menu"] } };
const radio: FakeNode = { role: "menuitemradio", parent: { role: "menu", class: ["ditox-menu"] } };
const search: FakeNode = { class: ["ditox-search"] };
const row: FakeNode = { class: ["ditox-row"] };

test("the three-dots button and the menu items activate themselves on Enter; the search and the rows do not", () => {
  for (const node of [more, item, radio, { parent: more }, { parent: item }, { parent: radio }]) {
    expect(activatesItself(fakeTarget(node))).toBe(true);
  }
  // The menu container is not a control, and the search input and the rows must keep Enter (they paste the selected row).
  for (const node of [search, row, { parent: row }, { role: "menu", class: ["ditox-menu"] }, {}]) {
    expect(activatesItself(fakeTarget(node))).toBe(false);
  }
  expect(activatesItself(null)).toBe(false);
  expect(activatesItself(undefined)).toBe(false);
  expect(activatesItself({} as EventTarget)).toBe(false); // window, document: no `closest`
  expect(SELF_ACTIVATING_TARGETS).toContain(".ditox-more");
});

test("the selector covers every role the menus render and the three-dots class stays in the footer", () => {
  const source = readFileSync(new URL("./Picker.tsx", import.meta.url), "utf8");
  const roles = [...new Set([...source.matchAll(/role="([^"]+)"/g)].map((match) => match[1]!))].filter((role) => role !== "menu");
  expect(roles).toContain("menuitem");
  expect(roles).toContain("menuitemradio");
  for (const role of roles) expect(activatesItself(fakeTarget({ role }))).toBe(true);
  const html = renderToStaticMarkup(
    <PickerView model={pickerModel(true, rows)} query="" onQuery={() => undefined} onPaste={() => undefined} />,
  );
  expect(html).toContain('class="ditox-more"');
  expect(html).toContain('class="ditox-search"');
  expect(html).toContain('class="ditox-row"');
});

test("Enter and Ctrl+Shift+V are left to the control that handles them; arrows and Escape are not", () => {
  expect(selectionAfterKey("Enter", false, false, fakeTarget(more))).toEqual({ kind: "none" });
  expect(selectionAfterKey("Enter", false, false, fakeTarget(item))).toEqual({ kind: "none" });
  expect(selectionAfterKey("Enter", false, false, fakeTarget(radio))).toEqual({ kind: "none" });
  expect(selectionAfterKey("V", true, true, fakeTarget(more))).toEqual({ kind: "none" });
  expect(selectionAfterKey("V", true, true, fakeTarget(item))).toEqual({ kind: "none" });
  // Everywhere else they paste the selected row.
  expect(selectionAfterKey("Enter", false, false, fakeTarget(search))).toEqual({ kind: "paste", mode: "original" });
  expect(selectionAfterKey("Enter", false, false, fakeTarget(row))).toEqual({ kind: "paste", mode: "original" });
  expect(selectionAfterKey("Enter", false, false, null)).toEqual({ kind: "paste", mode: "original" });
  expect(selectionAfterKey("Enter", false, false)).toEqual({ kind: "paste", mode: "original" });
  expect(selectionAfterKey("v", true, true, fakeTarget(search))).toEqual({ kind: "paste", mode: "plain" });
  // Navigation and closing still work from the three-dots button and the menu items.
  expect(selectionAfterKey("ArrowDown", false, false, fakeTarget(more))).toEqual({ kind: "move", delta: 1 });
  expect(selectionAfterKey("ArrowUp", false, false, fakeTarget(item))).toEqual({ kind: "move", delta: -1 });
  expect(selectionAfterKey("Escape", false, false, fakeTarget(radio))).toEqual({ kind: "hide" });
});

test("every preview resolves its own direction from its text (Arabic and Hebrew entries read right to left)", () => {
  const mixed: PickerEntry[] = [
    ...rows,
    { id: 4, kind: "text", mime: "text/plain", preview: "ar", content: "مرحبا Hello world (v2.0) — تجربة: ditox!" },
    { id: 5, kind: "image", mime: "image/png", preview: "صورة الشاشة 640x480", content: "hash" },
  ];
  const html = renderToStaticMarkup(
    <PickerView model={pickerModel(true, mixed)} query="" onQuery={() => undefined} onPaste={() => undefined} />,
  );
  // One auto-direction preview per row, text and image entries alike.
  expect(html.match(/<span class="ditox-preview" dir="auto">/g)?.length).toBe(mixed.length);
  expect(html.match(/class="ditox-preview"/g)?.length).toBe(mixed.length);
  expect(html).toContain("مرحبا Hello world (v2.0) — تجربة: ditox!");
  expect(html).toContain("صورة الشاشة 640x480");
});
