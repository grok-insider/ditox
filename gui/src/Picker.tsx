import { Button, CollectionToolbar, Input, PageHeader, StatePanel } from "@fabrials/ui";
import { filterEntries, shownImage, visiblePreview, type PasteMode, type PickerEntry, type PickerModel } from "./model";

export function PickerView(props: {
  model: PickerModel;
  query: string;
  imageSrc?: string | null;
  onQuery: (query: string) => void;
  onSelect?: (id: number) => void;
  onPaste: (id: number, mode: PasteMode) => void;
}) {
  if (props.model.status === "daemon-down") {
    return <StatePanel state="offline" title="The daemon is down." description="Start the clipboard daemon and open this window again." />;
  }
  if (props.model.status === "empty") {
    return <StatePanel state="empty" title="Empty history." description="Copies show up here after the daemon stores them." />;
  }
  const entries = filterEntries(props.model.entries, props.query);
  const selectedId = props.model.entries[props.model.selected]?.id;
  return (
    <section className="ditox-picker" data-density="compact">
      <PageHeader title="History" />
      <CollectionToolbar
        label="History"
        search={<Input aria-label="Search" value={props.query} onChange={(event) => props.onQuery(event.currentTarget.value)} />}
      />
      <ul>
        {entries.map((entry) => (
          <li key={entry.id}>
            <PickerRow
              entry={entry}
              selected={entry.id === selectedId}
              imageSrc={entry.kind === "image" && entry.id === selectedId ? props.imageSrc : null}
              onSelect={() => props.onSelect?.(entry.id)}
            />
            <Button type="button" onClick={() => props.onPaste(entry.id, "original")}>
              Paste
            </Button>
            <Button type="button" onClick={() => props.onPaste(entry.id, "plain")}>
              Plain
            </Button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function PickerRow(props: { entry: PickerEntry; selected: boolean; imageSrc?: string | null; onSelect: () => void }) {
  const src = props.selected ? shownImage(props.imageSrc) : null;
  return (
    <button type="button" className="ditox-row" data-selected={props.selected ? "true" : "false"} onClick={props.onSelect}>
      {visiblePreview(props.entry)}
      {src ? <img alt="" src={src} /> : null}
    </button>
  );
}
