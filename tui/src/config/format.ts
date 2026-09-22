import {
  defaultKeyLabels,
  defaultLabels,
  type KeyDisplayLabels,
  type ResolvedTuiConfig,
  type TuiHelpActionName,
  type TuiLabels,
  type TuiStatusHintMode,
} from "./types";

export function keyDisplay(keys: string[], labels: Pick<TuiLabels, "keyAlternativeSeparator"> = defaultLabels, keyLabels: KeyDisplayLabels = defaultKeyLabels): string {
  return keys.map((key) => displayKey(key, keyLabels)).join(labels.keyAlternativeSeparator);
}

export function paddedTitle(value: string, paddingLeft: number, paddingRight = paddingLeft): string {
  const left = " ".repeat(Math.max(0, Math.floor(paddingLeft)));
  const right = " ".repeat(Math.max(0, Math.floor(paddingRight)));
  return `${left}${value}${right}`;
}

export function helpRows(config: ResolvedTuiConfig): Array<{ keys: string; action: string }> {
  const keys = config.keyBindings;
  const labels = config.labels;
  const keyLabels = config.keyLabels;
  const rows: Record<TuiHelpActionName, { keys: string; action: string }> = {
    moveSelection: { keys: keyGroup(labels, keyLabels, keys.up, keys.down), action: labels.helpMoveSelection },
    pageSelection: { keys: keyGroup(labels, keyLabels, keys.pageUp, keys.pageDown), action: labels.helpPageSelection },
    firstLastEntry: { keys: keyGroup(labels, keyLabels, keys.home, keys.end), action: labels.helpFirstLastEntry },
    quit: { keys: keyGroup(labels, keyLabels, keys.quit, keys.forceQuit), action: labels.helpQuit },
    preview: { keys: keyDisplay(keys.preview, labels, keyLabels), action: labels.helpPreview },
    previewNavigation: {
      keys: keyGroup(labels, keyLabels, keys.previewUp, keys.previewDown, keys.previewPageUp, keys.previewPageDown),
      action: labels.helpPreviewNavigation,
    },
    previewBack: { keys: keyDisplay(keys.previewBack, labels, keyLabels), action: labels.helpPreviewBack },
    pinnedView: { keys: keyDisplay(keys.togglePinnedView, labels, keyLabels), action: labels.helpPinnedView },
    paste: { keys: keyDisplay(keys.copyPaste, labels, keyLabels), action: labels.helpPaste },
    pastePlain: { keys: keyDisplay(keys.pastePlain, labels, keyLabels), action: labels.helpPastePlain },
    copySet: { keys: keyGroup(labels, keyLabels, keys.copyOnly, keys.bulkCopy), action: labels.helpCopySet },
    output: { keys: keyDisplay(keys.output, labels, keyLabels), action: labels.helpOutput },
    markSingle: { keys: keyGroup(labels, keyLabels, keys.selectToggle, keys.selectSingle, keys.clearSelection), action: labels.helpMarkSingle },
    rangeSelect: { keys: keyGroup(labels, keyLabels, keys.selectUp, keys.selectDown), action: labels.helpRangeSelect },
    searchFilter: { keys: keyGroup(labels, keyLabels, keys.search, keys.nextFilter), action: labels.helpSearchFilter },
    searchEdit: {
      keys: keyGroup(labels, keyLabels, keys.searchBackspace, keys.searchApply, keys.searchCancel),
      action: labels.helpSearchEdit,
    },
    searchCopyMatches: { keys: keyDisplay(keys.searchCopyMatches, labels, keyLabels), action: labels.helpSearchCopyMatches },
    pinDelete: { keys: keyGroup(labels, keyLabels, keys.toggleFavorite, keys.delete), action: labels.helpPinDelete },
    clearHistory: { keys: keyGroup(labels, keyLabels, keys.clearAll, keys.clearText, keys.clearImages), action: labels.helpClearHistory },
    clearAllIncludingPinned: { keys: keyDisplay(keys.clearAllIncludingPinned, labels, keyLabels), action: labels.helpClearAllIncludingPinned },
    confirmChoice: { keys: keyGroup(labels, keyLabels, keys.confirmYes, keys.confirmNo), action: labels.helpConfirmChoice },
  };
  return config.helpOrder.map((name) => rows[name]);
}

export function statusHint(config: ResolvedTuiConfig, mode: TuiStatusHintMode = "browse"): string {
  const labels = config.labels;
  return formatTemplate(statusHintTemplate(labels, mode), statusHintReplacements(config));
}

// Keeps whole hint groups. A narrow status line used to end on a cut chord
// such as "preview /..." because the hint was one string clipped with an ellipsis.
export type StatusHintSpan = { role: "keys" | "action" | "gap"; text: string };

export function fittedStatusHint(config: ResolvedTuiConfig, width: number, mode: TuiStatusHintMode = "browse"): string {
  return fittedStatusHintSpans(config, width, mode).map((span) => span.text).join("");
}

export function fittedStatusHintSpans(config: ResolvedTuiConfig, width: number, mode: TuiStatusHintMode = "browse"): StatusHintSpan[] {
  const labels = config.labels;
  const replacements = statusHintReplacements(config);
  const collapse = (value: string) => collapseStatusWhitespace(value, labels.textWhitespaceReplacement);
  const separator = collapse(replacements.separator);
  const groups = statusHintTemplate(labels, mode)
    .split("{separator}")
    .map((group) => hintGroupSpans(group, replacements, collapse))
    .filter((group) => group.some((span) => span.text.length > 0));
  if (width <= 0 || groups.length === 0) return [];
  const textOf = (group: StatusHintSpan[]) => group.map((span) => span.text).join("");
  const full = groups.map(textOf).join(separator);
  const chosen = full.length <= width ? groups : groups.slice(0, fittingGroupCount(groups.map(textOf), separator, width));
  if (chosen.length === 0) {
    const clipped = truncateStatusHint(textOf(groups[0] ?? []), width, labels.textTruncationMarker);
    return clipped.length > 0 ? [{ role: "action", text: clipped }] : [];
  }
  const spans: StatusHintSpan[] = [];
  chosen.forEach((group, index) => {
    if (index > 0 && separator.length > 0) spans.push({ role: "gap", text: separator });
    spans.push(...group);
  });
  return spans;
}

function fittingGroupCount(groups: string[], separator: string, width: number): number {
  let count = 0;
  for (const group of groups) {
    const next = count === 0 ? group : `${groups.slice(0, count).join(separator)}${separator}${group}`;
    if (next.length > width) break;
    count += 1;
  }
  return count;
}

function hintGroupSpans(template: string, replacements: Record<string, string>, collapse: (value: string) => string): StatusHintSpan[] {
  const spans: StatusHintSpan[] = [];
  const pattern = /\{([a-zA-Z0-9_]+)\}/g;
  let cursor = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(template)) !== null) {
    if (match.index > cursor) pushHintSpan(spans, "action", collapse(template.slice(cursor, match.index)));
    const name = match[1] ?? "";
    pushHintSpan(spans, name.endsWith("Keys") ? "keys" : "action", collapse(replacements[name] ?? ""));
    cursor = match.index + match[0].length;
  }
  if (cursor < template.length) pushHintSpan(spans, "action", collapse(template.slice(cursor)));
  return spans;
}

function pushHintSpan(spans: StatusHintSpan[], role: "keys" | "action", text: string) {
  if (text.length === 0) return;
  spans.push({ role, text });
}

function statusHintReplacements(config: ResolvedTuiConfig): Record<string, string> {
  const keys = config.keyBindings;
  const labels = config.labels;
  const keyLabels = config.keyLabels;
  return {
    pasteKeys: keyDisplay(keys.copyPaste, labels, keyLabels),
    paste: labels.statusPasteHint,
    copyKeys: keyDisplay(keys.copyOnly, labels, keyLabels),
    copy: labels.statusCopyHint,
    previewKeys: keyDisplay(keys.preview, labels, keyLabels),
    preview: labels.statusPreviewHint,
    searchKeys: keyDisplay(keys.search, labels, keyLabels),
    search: labels.statusSearchHint,
    filterKeys: keyDisplay(keys.nextFilter, labels, keyLabels),
    filter: labels.statusFilterHint,
    pinnedKeys: keyDisplay(keys.togglePinnedView, labels, keyLabels),
    pinned: labels.statusPinnedHint,
    deleteKeys: keyDisplay(keys.delete, labels, keyLabels),
    delete: labels.statusDeleteHint,
    outputKeys: keyDisplay(keys.output, labels, keyLabels),
    output: labels.statusOutputHint,
    helpKeys: keyDisplay(keys.help, labels, keyLabels),
    help: labels.statusHelpHint,
    quitKeys: keyGroup(labels, keyLabels, keys.quit, keys.forceQuit),
    quit: labels.statusQuitHint,
    applyKeys: keyDisplay(keys.searchApply, labels, keyLabels),
    apply: labels.statusApplyHint,
    backspaceKeys: keyDisplay(keys.searchBackspace, labels, keyLabels),
    backspace: labels.statusBackspaceHint,
    cancelKeys: keyDisplay(keys.searchCancel, labels, keyLabels),
    cancel: labels.statusCancelHint,
    searchCopyKeys: keyDisplay(keys.searchCopyMatches, labels, keyLabels),
    searchCopy: labels.statusSearchCopyHint,
    previewBackKeys: keyDisplay(keys.previewBack, labels, keyLabels),
    previewBack: labels.statusPreviewBackHint,
    previewScrollKeys: keyGroup(labels, keyLabels, keys.previewUp, keys.previewDown, keys.previewPageUp, keys.previewPageDown),
    previewScroll: labels.statusPreviewScrollHint,
    confirmYesKeys: keyDisplay(keys.confirmYes, labels, keyLabels),
    confirmYes: labels.statusConfirmYesHint,
    confirmNoKeys: keyDisplay(keys.confirmNo, labels, keyLabels),
    confirmNo: labels.statusConfirmNoHint,
    separator: labels.statusHintSeparator,
  };
}

function collapseStatusWhitespace(value: string, replacement: string): string {
  const space = replacement.length > 0 ? replacement : " ";
  return value.replace(/\s+/g, space);
}

function truncateStatusHint(value: string, width: number, marker: string): string {
  if (width <= 0) return "";
  if (value.length <= width) return value;
  const ellipsis = marker.length > 0 ? marker : "...";
  if (width <= ellipsis.length) return ellipsis.slice(0, width);
  return `${value.slice(0, width - ellipsis.length)}${ellipsis}`;
}

function statusHintTemplate(labels: TuiLabels, mode: TuiStatusHintMode): string {
  if (mode === "search") return labels.statusSearchModeHintTemplate;
  if (mode === "preview") return labels.statusPreviewModeHintTemplate;
  if (mode === "confirm") return labels.statusConfirmModeHintTemplate;
  return labels.statusHintTemplate;
}

function keyGroup(labels: Pick<TuiLabels, "keyAlternativeSeparator" | "keyGroupSeparator">, keyLabels: KeyDisplayLabels, ...groups: string[][]): string {
  return groups.map((keys) => keyDisplay(keys, labels, keyLabels)).join(labels.keyGroupSeparator);
}

export function formatTemplate(value: string, replacements: Record<string, string | number>): string {
  let out = value;
  for (const [key, replacement] of Object.entries(replacements)) {
    out = out.replaceAll(`{${key}}`, String(replacement));
  }
  return out;
}

export type TemplateSegment = {
  key: string | null;
  text: string;
};

export function templateSegments(template: string, replacements: Record<string, string | number>): TemplateSegment[] {
  const parts: TemplateSegment[] = [];
  const pattern = /\{([a-zA-Z0-9_]+)\}/g;
  let cursor = 0;
  let match: RegExpExecArray | null;

  while ((match = pattern.exec(template)) !== null) {
    if (match.index > cursor) parts.push({ key: null, text: template.slice(cursor, match.index) });
    const key = match[1]!;
    if (Object.hasOwn(replacements, key)) {
      const text = String(replacements[key]);
      if (text.length > 0) parts.push({ key, text });
    } else {
      parts.push({ key: null, text: match[0] });
    }
    cursor = match.index + match[0].length;
  }

  if (cursor < template.length) parts.push({ key: null, text: template.slice(cursor) });
  return parts;
}

export function normalizeKey(key: string): string {
  if (key === " ") return "space";
  if (/^[A-Z]$/.test(key)) return `shift+${key.toLowerCase()}`;
  const parts = key.toLowerCase().split("+");
  if (parts[parts.length - 1] === "return") parts[parts.length - 1] = "enter";
  return parts.join("+");
}

function displayKey(key: string, labels: KeyDisplayLabels): string {
  const normalized = normalizeKey(key);
  return labels[normalized] ?? key;
}

