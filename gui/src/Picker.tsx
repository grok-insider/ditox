import { useEffect, useState } from "react";
import { shownImage, visiblePreview, type PasteMode, type PickerEntry, type PickerModel } from "./model";

const TRANSPARENCY = [0, 10, 20, 30] as const;

export function PickerView(props: {
  model: PickerModel;
  query: string;
  imageSrc?: string | null;
  previewLines?: 1 | 2;
  transparency?: number;
  onQuery: (query: string) => void;
  onSelect?: (id: number) => void;
  onPaste: (id: number, mode: PasteMode) => void;
  onTransparency?: (percent: number) => void;
  onOptions?: () => void;
}) {
  const [rowMenu, setRowMenu] = useState<{ id: number; x: number; y: number } | null>(null);
  const [moreMenu, setMoreMenu] = useState<{ x: number; y: number } | null>(null);
  const lines = props.previewLines === 1 ? 1 : 2;

  useEffect(() => {
    if (!rowMenu && !moreMenu) return;
    const close = () => {
      setRowMenu(null);
      setMoreMenu(null);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      close();
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [rowMenu, moreMenu]);

  if (props.model.status === "daemon-down") {
    return (
      <section className="ditox-picker" data-density="compact" data-lines={lines}>
        <p className="ditox-status">The daemon is down.</p>
      </section>
    );
  }

  const selectedId = props.model.status === "ready" ? props.model.entries[props.model.selected]?.id : undefined;
  const search = (
    <footer className="ditox-footer">
      <input
        className="ditox-search"
        aria-label="Search"
        placeholder="Search"
        value={props.query}
        autoFocus
        onChange={(event) => props.onQuery(event.currentTarget.value)}
      />
      <button
        type="button"
        className="ditox-more"
        aria-label="Menu"
        onMouseDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          const rect = event.currentTarget.getBoundingClientRect();
          setRowMenu(null);
          setMoreMenu({ x: Math.max(8, rect.right - 180), y: Math.max(8, rect.top - 8) });
        }}
      >
        …
      </button>
    </footer>
  );

  return (
    <section className="ditox-picker" data-density="compact" data-lines={lines}>
      {props.model.status === "ready" ? (
        <ul className="ditox-list">
          {props.model.entries.map((entry) => (
            <li key={entry.id}>
              <PickerRow
                entry={entry}
                selected={entry.id === selectedId}
                imageSrc={entry.kind === "image" && entry.id === selectedId ? props.imageSrc : null}
                onSelect={() => props.onSelect?.(entry.id)}
                onPaste={() => props.onPaste(entry.id, "original")}
                onMenu={(x, y) => {
                  props.onSelect?.(entry.id);
                  setMoreMenu(null);
                  setRowMenu({
                    id: entry.id,
                    x: Math.min(x, window.innerWidth - 168),
                    y: Math.min(y, window.innerHeight - 72),
                  });
                }}
              />
            </li>
          ))}
        </ul>
      ) : (
        <p className="ditox-status">{props.model.status === "empty" ? "Empty history." : "No results."}</p>
      )}
      {search}
      {rowMenu ? (
        <div className="ditox-menu" role="menu" style={{ top: rowMenu.y, left: rowMenu.x }} onMouseDown={(event) => event.stopPropagation()}>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const id = rowMenu.id;
              setRowMenu(null);
              props.onPaste(id, "original");
            }}
          >
            Paste
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const id = rowMenu.id;
              setRowMenu(null);
              props.onPaste(id, "plain");
            }}
          >
            Paste as text
          </button>
        </div>
      ) : null}
      {moreMenu ? (
        <div
          className="ditox-menu ditox-menu-up"
          role="menu"
          style={{ left: moreMenu.x, top: moreMenu.y }}
          onMouseDown={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            disabled={selectedId == null}
            onClick={() => {
              setMoreMenu(null);
              if (selectedId != null) props.onPaste(selectedId, "original");
            }}
          >
            Paste
          </button>
          <button
            type="button"
            role="menuitem"
            disabled={selectedId == null}
            onClick={() => {
              setMoreMenu(null);
              if (selectedId != null) props.onPaste(selectedId, "plain");
            }}
          >
            Paste as text
          </button>
          {TRANSPARENCY.map((percent) => (
            <button
              key={percent}
              type="button"
              role="menuitemradio"
              aria-checked={props.transparency === percent}
              onClick={() => {
                setMoreMenu(null);
                props.onTransparency?.(percent);
              }}
            >
              {percent === 0 ? "Opaque" : `${percent}%`}
            </button>
          ))}
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setMoreMenu(null);
              props.onOptions?.();
            }}
          >
            Options
          </button>
        </div>
      ) : null}
    </section>
  );
}

function PickerRow(props: {
  entry: PickerEntry;
  selected: boolean;
  imageSrc?: string | null;
  onSelect: () => void;
  onPaste: () => void;
  onMenu: (x: number, y: number) => void;
}) {
  const src = props.selected ? shownImage(props.imageSrc) : null;
  return (
    <button
      type="button"
      className="ditox-row"
      data-selected={props.selected ? "true" : "false"}
      onClick={props.onSelect}
      onDoubleClick={props.onPaste}
      onContextMenu={(event) => {
        event.preventDefault();
        props.onMenu(event.clientX, event.clientY);
      }}
    >
      {/* dir="auto": clipboard text is not in the UI's language. The first strong character decides the direction of the whole entry. */}
      <span className="ditox-preview" dir="auto">
        {visiblePreview(props.entry)}
      </span>
      {src ? <img alt="" src={src} /> : null}
    </button>
  );
}
