import { useEffect, useState } from "react";
import { filterEntries, shownImage, visiblePreview, type PasteMode, type PickerEntry, type PickerModel } from "./model";

export function PickerView(props: {
  model: PickerModel;
  query: string;
  imageSrc?: string | null;
  onQuery: (query: string) => void;
  onSelect?: (id: number) => void;
  onPaste: (id: number, mode: PasteMode) => void;
}) {
  const [menu, setMenu] = useState<{ id: number; x: number; y: number } | null>(null);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      setMenu(null);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [menu]);

  if (props.model.status === "daemon-down") {
    return (
      <section className="ditox-picker" data-density="compact">
        <p className="ditox-status">The daemon is down.</p>
      </section>
    );
  }

  const search = (
    <input
      className="ditox-search"
      aria-label="Search"
      placeholder="Search"
      value={props.query}
      autoFocus
      onChange={(event) => props.onQuery(event.currentTarget.value)}
    />
  );

  if (props.model.status === "empty") {
    return (
      <section className="ditox-picker" data-density="compact">
        {search}
        <p className="ditox-status">Empty history.</p>
      </section>
    );
  }

  const entries = filterEntries(props.model.entries, props.query);
  const selectedId = props.model.entries[props.model.selected]?.id;
  return (
    <section className="ditox-picker" data-density="compact">
      {search}
      <ul className="ditox-list">
        {entries.map((entry) => (
          <li key={entry.id}>
            <PickerRow
              entry={entry}
              selected={entry.id === selectedId}
              imageSrc={entry.kind === "image" && entry.id === selectedId ? props.imageSrc : null}
              onSelect={() => props.onSelect?.(entry.id)}
              onPaste={() => props.onPaste(entry.id, "original")}
              onMenu={(x, y) => {
                props.onSelect?.(entry.id);
                setMenu({
                  id: entry.id,
                  x: Math.min(x, window.innerWidth - 168),
                  y: Math.min(y, window.innerHeight - 44),
                });
              }}
            />
          </li>
        ))}
      </ul>
      {menu ? (
        <div
          className="ditox-menu"
          role="menu"
          style={{ top: menu.y, left: menu.x }}
          onMouseDown={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const id = menu.id;
              setMenu(null);
              props.onPaste(id, "plain");
            }}
          >
            Paste as text
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
      <span className="ditox-preview">{visiblePreview(props.entry)}</span>
      {src ? <img alt="" src={src} /> : null}
    </button>
  );
}
