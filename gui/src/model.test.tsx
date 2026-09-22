import { expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { PickerView } from "./Picker";
import {
  deliveryRequest,
  entriesFromList,
  filterEntries,
  imagePreviewSrc,
  moveSelection,
  pasteRequest,
  pasteSelection,
  pickerModel,
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
  const html = renderToStaticMarkup(
    <PickerView model={pickerModel(true, rows)} query="example.com" onQuery={() => undefined} onPaste={() => undefined} />,
  );
  expect(html).toContain("https://example.com/clip");
  expect(html).not.toContain("hello clipboard");
  expect(html).not.toContain(">Paste<");
  expect(html).not.toContain(">Plain<");
  expect(html).not.toContain("History");
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
