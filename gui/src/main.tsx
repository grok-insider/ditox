import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "@fabrials/ui/tokens.css";
import "@fabrials/ui/styles.css";
import "@fabrials/ui/fonts.css";
import "./picker.css";
import { PickerView } from "./Picker";
import {
  imagePreviewSrc,
  moveSelection,
  pasteSelection,
  refreshPicker,
  selectEntry,
  selectionAfterKey,
  transparencyAlpha,
  type ImagePayload,
  type ListCommands,
  type PasteMode,
  type PickerModel,
  type RpcRequest,
} from "./model";

type ShellCommands = ListCommands & {
  image: (id: number) => Promise<ImagePayload | null>;
  paste: (request: RpcRequest) => Promise<void>;
  target: () => Promise<string | null>;
  hide: () => Promise<void>;
};

const commands: ShellCommands = {
  status: () => invoke<string>("daemon_status"),
  list: () => invoke("list_entries", { query: "" }),
  image: async (id) => {
    try {
      return await invoke<ImagePayload>("entry_image", { id });
    } catch {
      return null;
    }
  },
  paste: (request) =>
    invoke("paste_entry", {
      id: request.params.id,
      mode: request.params.mode,
      ...(request.params.target_window ? { target_window: request.params.target_window } : {}),
    }),
  target: () => invoke<string | null>("captured_window"),
  hide: () => invoke("hide_window"),
};

document.documentElement.classList.add("dark");

function applyTransparency(percent: number) {
  document.documentElement.style.setProperty("--ditox-alpha", String(transparencyAlpha(percent)));
}

createRoot(document.getElementById("root")!).render(<PickerApp commands={commands} />);

export function PickerApp(props: { commands: ShellCommands }) {
  const [model, setModel] = useState<PickerModel>({ status: "daemon-down" });
  const [query, setQuery] = useState("");
  const [imageSrc, setImageSrc] = useState<string | null>(null);
  const modelRef = useRef(model);
  modelRef.current = model;

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      const next = await refreshPicker(props.commands, modelRef.current);
      if (!cancelled) setModel(next);
    };
    void load();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, 1000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [props.commands]);

  useEffect(() => {
    const entry = model.status === "ready" ? model.entries[model.selected] : undefined;
    if (!entry || entry.kind !== "image") {
      setImageSrc(null);
      return;
    }
    let cancelled = false;
    void props.commands.image(entry.id).then((image) => {
      if (cancelled) return;
      setImageSrc(image ? imagePreviewSrc(image.mime, image.data) : null);
    });
    return () => {
      cancelled = true;
    };
  }, [props.commands, model]);

  useEffect(() => {
    let cancel = false;
    let unlistenPaste: (() => void) | undefined;
    let unlistenAlpha: (() => void) | undefined;
    void invoke<number>("window_transparency")
      .then((percent) => {
        if (!cancel) applyTransparency(percent);
      })
      .catch(() => undefined);
    void listen<number>("ditox-transparency", (event) => {
      applyTransparency(event.payload);
    }).then((unlisten) => {
      if (cancel) unlisten();
      else unlistenAlpha = unlisten;
    });
    void listen<string>("ditox-paste", (event) => {
      const mode: PasteMode = event.payload === "plain" ? "plain" : "original";
      const current = modelRef.current;
      if (current.status !== "ready") return;
      const entry = current.entries[current.selected] ?? current.entries[0];
      if (!entry) return;
      void pasteSelection(props.commands, entry.id, mode);
    }).then((unlisten) => {
      if (cancel) unlisten();
      else unlistenPaste = unlisten;
    });
    return () => {
      cancel = true;
      unlistenPaste?.();
      unlistenAlpha?.();
    };
  }, [props.commands]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const action = selectionAfterKey(event.key, event.ctrlKey, event.shiftKey);
      if (action.kind === "none") return;
      event.preventDefault();
      if (action.kind === "move") {
        setModel((current) => moveSelection(current, action.delta));
        return;
      }
      if (action.kind === "hide") {
        void props.commands.hide();
        return;
      }
      const current = modelRef.current;
      if (current.status !== "ready") return;
      const entry = current.entries[current.selected];
      if (!entry) return;
      void pasteSelection(props.commands, entry.id, action.mode);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [props.commands]);

  return (
    <PickerView
      model={model}
      query={query}
      imageSrc={imageSrc}
      onQuery={setQuery}
      onSelect={(id) => setModel((current) => selectEntry(current, id))}
      onPaste={(id, mode: PasteMode) => void pasteSelection(props.commands, id, mode)}
    />
  );
}
