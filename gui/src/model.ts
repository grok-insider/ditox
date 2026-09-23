export type PickerEntry = {
  id: number;
  kind: "text" | "image";
  mime: string;
  preview: string;
  content: string;
};

export type PickerModel =
  | { status: "daemon-down" }
  | { status: "empty" }
  | { status: "no-results"; query: string }
  | { status: "ready"; entries: PickerEntry[]; selected: number };

export type PasteMode = "original" | "plain";

export type RpcRequest = {
  method: "entries.paste";
  params: { id: number; mode: PasteMode; target_window?: string };
};

export type KeyAction =
  | { kind: "move"; delta: number }
  | { kind: "paste"; mode: PasteMode }
  | { kind: "hide" }
  | { kind: "none" };

export type ListCommands = {
  status: () => Promise<string>;
  list: (query: string) => Promise<unknown>;
};

export type ImagePayload = { mime: string; data: string };

export function pickerModel(connected: boolean, entries: PickerEntry[], selected = 0, query = ""): PickerModel {
  if (!connected) return { status: "daemon-down" };
  if (entries.length === 0) {
    const needle = query.trim();
    return needle.length === 0 ? { status: "empty" } : { status: "no-results", query: needle };
  }
  const index = Math.min(Math.max(0, selected), entries.length - 1);
  return { status: "ready", entries, selected: index };
}

export function listRequest(query: string): { query: string; limit: number } {
  return { query: query.trim(), limit: 1000 };
}

export function previewLines(value: number): 1 | 2 {
  return value === 1 ? 1 : 2;
}

export function entriesFromList(payload: unknown): PickerEntry[] {
  return listedEntries(payload).flatMap((item) => {
    if (!item || typeof item !== "object") return [];
    const row = item as Record<string, unknown>;
    if (typeof row.id !== "number") return [];
    if (row.kind !== "text" && row.kind !== "image") return [];
    return [
      {
        id: row.id,
        kind: row.kind,
        mime: typeof row.mime === "string" ? row.mime : "",
        preview: typeof row.preview === "string" ? row.preview : "",
        content: typeof row.content === "string" ? row.content : "",
      },
    ];
  });
}

export function filterEntries(entries: PickerEntry[], query: string): PickerEntry[] {
  const needle = query.trim().toLowerCase();
  if (needle.length === 0) return entries;
  return entries.filter((entry) => `${entry.preview}\n${entry.content}`.toLowerCase().includes(needle));
}

export function visiblePreview(entry: PickerEntry): string {
  return entry.kind === "image" ? entry.preview : entry.content;
}

export function imagePreviewSrc(mime: string, data: string): string | null {
  if (mime !== "image/png" && mime !== "image/jpeg") return null;
  if (!isStandardBase64(data)) return null;
  return `data:${mime};base64,${data}`;
}

export function shownImage(src: string | null | undefined): string | null {
  if (!src) return null;
  const marker = ";base64,";
  const split = src.indexOf(marker);
  if (!src.startsWith("data:image/png;") && !src.startsWith("data:image/jpeg;")) return null;
  if (split < 0) return null;
  if (!isStandardBase64(src.slice(split + marker.length))) return null;
  return src;
}

function isStandardBase64(data: string): boolean {
  return /^[A-Za-z0-9+/]+={0,2}$/.test(data);
}

export function pasteRequest(id: number, mode: PasteMode, targetWindow?: string): RpcRequest {
  return {
    method: "entries.paste",
    params: { id, mode, target_window: targetWindow },
  };
}

export function deliveryRequest(id: number, mode: PasteMode, targetWindow?: string | null): RpcRequest {
  const target = targetWindow?.trim();
  if (target) return pasteRequest(id, mode, target);
  return { method: "entries.paste", params: { id, mode } };
}

export function moveSelection(model: PickerModel, delta: number): PickerModel {
  if (model.status !== "ready") return model;
  return pickerModel(true, model.entries, model.selected + delta);
}

export function selectEntry(model: PickerModel, id: number): PickerModel {
  if (model.status !== "ready") return model;
  const index = model.entries.findIndex((entry) => entry.id === id);
  if (index < 0) return model;
  return pickerModel(true, model.entries, index);
}

export function transparencyAlpha(percent: number): number {
  if (percent === 10) return 0.9;
  if (percent === 20) return 0.8;
  if (percent === 30) return 0.7;
  return 1;
}

export function selectionAfterKey(key: string, ctrl: boolean, shift: boolean): KeyAction {
  if (key === "ArrowDown") return { kind: "move", delta: 1 };
  if (key === "ArrowUp") return { kind: "move", delta: -1 };
  if (key === "Enter") return { kind: "paste", mode: "original" };
  if (ctrl && shift && key.toLowerCase() === "v") return { kind: "paste", mode: "plain" };
  if (key === "Escape") return { kind: "hide" };
  return { kind: "none" };
}

export async function refreshPicker(commands: ListCommands, previous: PickerModel, query = ""): Promise<PickerModel> {
  try {
    const status = await commands.status();
    if (status !== "connect") return pickerModel(false, [], 0, query);
    const entries = entriesFromList(await commands.list(listRequest(query).query));
    const previousId = previous.status === "ready" ? previous.entries[previous.selected]?.id : undefined;
    const index = previousId == null ? 0 : entries.findIndex((entry) => entry.id === previousId);
    return pickerModel(true, entries, index < 0 ? 0 : index, query);
  } catch {
    return pickerModel(false, [], 0, query);
  }
}

export async function pasteSelection(
  commands: {
    target: () => Promise<string | null | undefined>;
    paste: (request: RpcRequest) => Promise<void>;
    hide: () => Promise<void>;
  },
  id: number,
  mode: PasteMode,
): Promise<void> {
  const target = await commands.target();
  await commands.paste(deliveryRequest(id, mode, target));
  await commands.hide();
}

function listedEntries(payload: unknown): unknown[] {
  if (!payload || typeof payload !== "object") return [];
  const body = payload as { result?: unknown; entries?: unknown };
  const result = body.result && typeof body.result === "object" ? (body.result as { entries?: unknown }) : null;
  const entries = result?.entries ?? body.entries;
  return Array.isArray(entries) ? entries : [];
}
