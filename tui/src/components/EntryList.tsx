import { For, Show } from "solid-js";
import { contentRowLayout, entryAccent, entryKindLabel, entryMeta, entryPreviewSegments, fitRowMeta, rowTrailing, scrollbarCells, visibleHistoryRows, type HistoryRow } from "../presentation";
import { listInteriorRows } from "../layout";
import { visibleEntryCapacity } from "../state";
import type { ContentAlign } from "../ui-config";
import {
  defaultLabels,
  formatTemplate,
  paddedTitle,
  surface,
  textStyle,
  type ResolvedTuiConfig,
  type TuiListContentPartName,
  type TuiSurfaceStyle,
} from "../tui-config";
import type { Entry } from "../types";
import { takeWheel } from "../mouse";
import { configuredToneColor, fitCell, justifyContent, verticalJustify } from "./style-utils";

export function EntryList(props: {
  config: ResolvedTuiConfig;
  entries: Entry[];
  selectedIndex: number;
  selectedIds: Set<number>;
  rows: number;
  width: number;
  widthPercent?: number;
  query: string;
  onSelectEntry: (index: number, options: { extend: boolean; toggle: boolean }) => void;
  onScroll: (direction: -1 | 1) => void;
}) {
  const style = () => surface(props.config, "list");
  const rowSpacerStyle = () => surface(props.config, "rowSpacer");
  const layout = () => props.config.layout;
  const chrome = () => props.config.chrome;
  const interiorRows = () => listInteriorRows(props.config, props.rows);
  const visibleCount = () => visibleEntryCapacity(interiorRows(), layout().rowSpacing);
  const visible = () =>
    visibleHistoryRows(
      props.entries,
      props.selectedIndex,
      visibleCount(),
      { pinned: props.config.labels.pinnedViewTitle },
      { groupByAge: props.query.trim().length === 0 },
    );
  return (
    <box
      width={`${props.widthPercent ?? layout().listWidthPercent}%`}
      border={chrome().listBorder ? true : undefined}
      borderStyle={chrome().listBorder ? chrome().listBorderStyle : undefined}
      borderColor={chrome().listBorder ? style().border : undefined}
      backgroundColor={style().bg}
      paddingX={layout().listPaddingX}
      paddingY={layout().listPaddingY}
      title={
        chrome().listBorder && chrome().showListTitle
          ? paddedTitle(props.config.labels.historyTitle, layout().frameTitlePaddingLeft, layout().frameTitlePaddingRight)
          : undefined
      }
      titleAlignment={chrome().listTitleAlignment}
      bottomTitle={
        chrome().listBorder && chrome().showListPositionTitle && props.entries.length > visibleCount()
          ? paddedTitle(
              listPositionTitle(props.config, props.selectedIndex, props.entries.length),
              layout().frameTitlePaddingLeft,
              layout().frameTitlePaddingRight,
            )
          : undefined
      }
      bottomTitleAlignment={chrome().listBottomTitleAlignment}
      onMouseScroll={(event: any) => {
        const direction = takeWheel(event);
        if (!direction) return;
        event.preventDefault();
        event.stopPropagation();
        props.onScroll(direction);
      }}
    >
      <Show
        when={props.entries.length > 0}
        fallback={<EmptyList config={props.config} query={props.query} />}
      >
        <box flexDirection="row" width="100%" height="100%">
          <Show when={layout().showScrollbar && layout().scrollbarPlacement === "left"}>
            <Scrollbar config={props.config} total={props.entries.length} selectedIndex={props.selectedIndex} rows={interiorRows()} />
          </Show>
          <box flexDirection="column" flexGrow={1}>
            <For each={visible()}>
              {(row, index) => (
                <>
                  <HistoryLine
                    config={props.config}
                    row={row}
                    width={props.width}
                    query={props.query}
                    selected={row.kind === "entry" && row.index === props.selectedIndex}
                    marked={row.kind === "entry" && props.selectedIds.has(row.entry.id)}
                    onSelectEntry={props.onSelectEntry}
                  />
                  <Show when={layout().rowSpacing > 0 && index() < visible().length - 1}>
                    <box width="100%" height={layout().rowSpacing} backgroundColor={rowSpacerStyle().bg} />
                  </Show>
                </>
              )}
            </For>
          </box>
          <Show when={layout().showScrollbar && layout().scrollbarPlacement === "right"}>
            <Scrollbar config={props.config} total={props.entries.length} selectedIndex={props.selectedIndex} rows={interiorRows()} />
          </Show>
        </box>
      </Show>
    </box>
  );
}

function HistoryLine(props: {
  config: ResolvedTuiConfig;
  row: HistoryRow;
  width: number;
  query: string;
  selected: boolean;
  marked: boolean;
  onSelectEntry: (index: number, options: { extend: boolean; toggle: boolean }) => void;
}) {
  const style = () => surface(props.config, "list");
  if (props.row.kind === "divider") {
    return (
      <box height={1} width="100%" backgroundColor={style().bg}>
        <text style={textStyle(style(), style().muted)}>{props.row.title}</text>
      </box>
    );
  }
  const entry = props.row.entry;
  const index = props.row.index;
  return (
    <EntryRow
      config={props.config}
      entry={entry}
      index={index}
      width={props.width}
      query={props.query}
      selected={props.selected}
      marked={props.marked}
      onMouseDown={(event: any) => {
        const options = entryMouseSelectionOptions(event);
        if (!options) return;
        event.preventDefault();
        props.onSelectEntry(index, options);
      }}
    />
  );
}

function EntryRow(props: {
  config: ResolvedTuiConfig;
  entry: Entry;
  index: number;
  width: number;
  query: string;
  selected: boolean;
  marked: boolean;
  onMouseDown: (event: any) => void;
}) {
  const rowStyle = () => {
    if (props.selected && props.marked) return surface(props.config, "selectedMarkedRow");
    if (props.selected) return surface(props.config, "selectedRow");
    if (props.marked) return surface(props.config, "markedRow");
    if (props.config.layout.alternateRows && props.index % 2 === 1) return surface(props.config, "alternateRow");
    return surface(props.config, "list");
  };
  const bg = () => rowStyle().bg;
  const marker = () =>
    props.selected && props.marked
      ? props.config.chrome.selectedMarkedMarker
      : props.selected
        ? props.config.chrome.selectedMarker
        : props.marked
          ? props.config.chrome.markedMarker
          : props.config.chrome.normalMarker;
  const layout = () => props.config.layout;
  const markerCellWidth = () => {
    const configured = Math.max(0, Math.floor(layout().rowMarkerWidth));
    return configured > 0 ? configured : Array.from(marker()).length;
  };
  const rowPrefixWidth = () => markerCellWidth() + layout().rowMarkerGap;
  const reservedRowMetaWidth = () => Math.max(rowPrefixWidth() + layout().rowMetaPreviewGap, layout().rowPreviewReservedWidth);
  const metadataWidth = () => Math.max(0, reservedRowMetaWidth() - rowPrefixWidth() - layout().rowMetaPreviewGap);
  const availablePreviewWidth = () => Math.max(1, props.width - (layout().showRowMetadata ? reservedRowMetaWidth() : rowPrefixWidth()));
  const highlightQuery = () => (layout().highlightSearchMatches ? props.query : "");
  const previewWidth = () => {
    const maxWidth = layout().rowPreviewMaxWidth;
    return maxWidth > 0 ? Math.min(availablePreviewWidth(), maxWidth) : availablePreviewWidth();
  };
  const previewSegments = () => entryPreviewSegments(props.entry, previewWidth(), highlightQuery(), props.config.labels);
  const previewPadding = () => alignmentPadding(layout().rowPreviewAlign, previewWidth(), previewSegments().reduce((length, segment) => length + (segment?.text?.length ?? 0), 0));
  const contentFirst = () => props.config.labels.rowMetaTemplate === defaultLabels.rowMetaTemplate;
  const contentFit = () => {
    const border = props.config.chrome.listBorder ? 2 : 0;
    const padding = props.config.layout.listPaddingX * 2;
    const scrollbar = props.config.layout.showScrollbar ? props.config.layout.scrollbarWidth : 0;
    const available = Math.max(1, props.width - border - padding - scrollbar - rowPrefixWidth());
    if (!layout().showRowMetadata) return { kind: "", trailing: "", previewWidth: available };
    return contentRowLayout(available, entryKindLabel(props.entry, props.config.labels), rowTrailing(props.entry, Date.now(), props.config.labels));
  };
  const contentPreviewWidth = () => {
    const cap = layout().rowPreviewMaxWidth;
    const fitted = contentFit().previewWidth;
    return cap > 0 ? Math.min(fitted, cap) : fitted;
  };
  const contentSegments = () => entryPreviewSegments(props.entry, contentPreviewWidth(), highlightQuery(), props.config.labels);
  if (contentFirst()) {
    return (
      <box height={1} width="100%" flexDirection="row" backgroundColor={bg()} onMouseDown={props.onMouseDown}>
        <text style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "preview"))}>
          <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "marker"))}>
            {fitCell(marker(), markerCellWidth(), layout().rowMarkerAlign)}
          </span>
          <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "markerGap"))}>{" ".repeat(layout().rowMarkerGap)}</span>
          <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "metadata"))}>
            {layout().showRowMetadata ? contentFit().kind : ""}
          </span>
          <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "metadataGap"))}>
            {layout().showRowMetadata && contentFit().kind.length > 0 ? " " : ""}
          </span>
          <For each={contentSegments()}>
            {(segment) => (
              <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, segment.match ? "searchMatch" : "preview"))}>
                {segment.text}
              </span>
            )}
          </For>
          <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "metadataGap"))}>
            {layout().showRowMetadata && contentFit().trailing.length > 0 ? ` ${contentFit().trailing}` : ""}
          </span>
        </text>
      </box>
    );
  }
  return (
    <box height={1} width="100%" flexDirection="row" justifyContent={justifyContent(layout().rowContentAlign)} backgroundColor={bg()} onMouseDown={props.onMouseDown}>
      <text style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "preview"))}>
        <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "marker"))}>
          {fitCell(marker(), markerCellWidth(), layout().rowMarkerAlign)}
        </span>
        <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "markerGap"))}>{" ".repeat(layout().rowMarkerGap)}</span>
        <Show when={layout().showRowMetadata}>
          <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "metadata"))}>
            {fitRowMeta(entryMeta(props.entry, Date.now(), props.config.labels, layout()), metadataWidth(), props.config.labels, layout().rowMetadataAlign)}
          </span>
          <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "metadataGap"))}>{" ".repeat(layout().rowMetaPreviewGap)}</span>
        </Show>
        <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, "preview"))}>{" ".repeat(previewPadding())}</span>
        <For each={previewSegments()}>
          {(segment) => (
            <span style={textStyle(rowStyle(), listContentColor(props.config, rowStyle(), props.entry, segment.match ? "searchMatch" : "preview"))}>
              {segment.text}
            </span>
          )}
        </For>
      </text>
    </box>
  );
}

function listPositionTitle(config: ResolvedTuiConfig, selectedIndex: number, total: number): string {
  return formatTemplate(config.labels.listPositionTemplate, {
    index: selectedIndex + 1,
    total,
  });
}

export function entryMouseSelectionOptions(event: any): { extend: boolean; toggle: boolean } | null {
  const button = mouseButton(event?.button);
  if (button !== 0 && button !== 2) return null;
  return {
    extend: button === 0 && Boolean(event?.modifiers?.shift),
    toggle: button === 2 || Boolean(event?.modifiers?.ctrl),
  };
}

function mouseButton(value: unknown): number | null {
  if (typeof value === "number") return value;
  if (value === "left") return 0;
  if (value === "right") return 2;
  return null;
}

function Scrollbar(props: { config: ResolvedTuiConfig; total: number; selectedIndex: number; rows: number }) {
  const style = () => surface(props.config, "scrollbar");
  const width = () => props.config.layout.scrollbarWidth;
  return (
    <box width={width()} flexDirection="column" backgroundColor={style().bg}>
      <For each={scrollbarCells(props.total, props.selectedIndex, props.rows, props.config.chrome.scrollbarThumb, props.config.chrome.scrollbarTrack)}>
        {(cell) => (
          <text style={textStyle(style(), listContentColor(props.config, style(), null, cell === props.config.chrome.scrollbarThumb ? "scrollbarThumb" : "scrollbarTrack"))}>
            {scrollbarCell(cell, width(), props.config.layout.scrollbarAlign)}
          </text>
        )}
      </For>
    </box>
  );
}

function scrollbarCell(value: string, width: number, align: ContentAlign): string {
  return fitCell(value, width, align);
}

function EmptyList(props: { config: ResolvedTuiConfig; query: string }) {
  const style = () => surface(props.config, "emptyState");
  const labels = () => props.config.labels;
  const layout = () => props.config.layout;
  return (
    <box
      width="100%"
      height="100%"
      flexDirection="column"
      justifyContent={verticalJustify(layout().emptyStateVerticalAlign)}
      paddingX={layout().emptyStatePaddingX}
      paddingY={layout().emptyStatePaddingY}
      backgroundColor={style().bg}
    >
      <box width="100%" flexDirection="row" justifyContent={justifyContent(layout().emptyStateTitleAlign)}>
        <text style={textStyle(style(), listContentColor(props.config, style(), null, "emptyTitle"))}>{props.query ? labels().noMatchesTitle : labels().noHistoryTitle}</text>
      </box>
      <Show when={layout().showEmptyStateHelp}>
        <>
          <Show when={layout().emptyStateLineSpacing > 0}>
            <box width="100%" height={layout().emptyStateLineSpacing} backgroundColor={style().bg} />
          </Show>
          <box width="100%" flexDirection="row" justifyContent={justifyContent(layout().emptyStateHelpAlign)}>
            <text style={textStyle(style(), listContentColor(props.config, style(), null, "emptyHelp"))}>
              {props.query ? labels().noMatchesHelp : labels().noHistoryHelp}
            </text>
          </box>
        </>
      </Show>
    </box>
  );
}

function alignmentPadding(align: ContentAlign, width: number, contentWidth: number): number {
  const extra = Math.max(0, Math.floor(width) - contentWidth);
  if (align === "right") return extra;
  if (align === "center") return Math.floor(extra / 2);
  return 0;
}

function listContentColor(config: ResolvedTuiConfig, style: TuiSurfaceStyle, entry: Entry | null, part: TuiListContentPartName): string {
  return configuredToneColor(style, config.listContentTones[part], autoListContentColor(style, entry, part));
}

function autoListContentColor(style: TuiSurfaceStyle, entry: Entry | null, part: TuiListContentPartName): string {
  switch (part) {
    case "marker":
    case "metadata":
      return entry ? entryAccent(style, entry) : style.accent;
    case "markerGap":
    case "metadataGap":
    case "emptyHelp":
    case "scrollbarTrack":
      return style.muted;
    case "searchMatch":
      return style.search;
    case "scrollbarThumb":
      return style.accent;
    case "emptyTitle":
    case "preview":
    default:
      return style.fg;
  }
}
