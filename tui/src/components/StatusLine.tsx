import { For } from "solid-js";
import { statusTone, truncateText, watcherStatusView } from "../presentation";
import {
  paddedTitle,
  fittedStatusHint,
  fittedStatusHintSpans,
  surface,
  templateSegments,
  textStyle,
  type ResolvedTuiConfig,
  type TuiStatusHintMode,
  type TuiSurfaceStyle,
} from "../tui-config";
import type { WatcherStatus } from "../types";
import { configuredMax, configuredToneColor, justifyContent, templateWidth, verticalJustify } from "./style-utils";

export function StatusLine(props: { config: ResolvedTuiConfig; status: string; watcher: WatcherStatus | null; width: number; mode?: TuiStatusHintMode }) {
  const style = () => surface(props.config, "status");
  const watcher = () => watcherStatusView(props.watcher, props.config.labels);
  const operationTone = () => statusTone(props.status, props.config.labels, props.config.statusTones);
  const watcherTone = () => watcher().tone;
  const operationText = () => props.status || props.config.labels.ready;
  const separatorText = () =>
    `${" ".repeat(props.config.layout.statusSeparatorPaddingLeft)}${props.config.chrome.statusSeparator}${" ".repeat(props.config.layout.statusSeparatorPaddingRight)}`;
  const lineValues = () =>
    fittedStatusLineValues(
      props.config,
      {
        separator: separatorText(),
        watcher: watcher().text,
        operation: operationText(),
      },
      Math.max(0, props.width - props.config.layout.statusPaddingX * 2),
      props.mode ?? "browse",
    );
  const lineParts = () => templateSegments(props.config.labels.statusLineTemplate, lineValues());
  const hintSpans = () => fittedStatusHintSpans(props.config, lineValues().hint.length, props.mode ?? "browse");
  return (
    <box
      height={props.config.layout.statusHeight}
      border={props.config.chrome.statusBorder ? true : undefined}
      borderStyle={props.config.chrome.statusBorder ? props.config.chrome.statusBorderStyle : undefined}
      borderColor={props.config.chrome.statusBorder ? style().border : undefined}
      backgroundColor={style().bg}
      paddingX={props.config.layout.statusPaddingX}
      paddingY={props.config.layout.statusPaddingY}
      title={
        props.config.chrome.statusBorder && props.config.chrome.showStatusTitle
          ? paddedTitle(props.config.labels.statusTitle, props.config.layout.frameTitlePaddingLeft, props.config.layout.frameTitlePaddingRight)
          : undefined
      }
      titleAlignment={props.config.chrome.statusTitleAlignment}
    >
      <box width="100%" flexGrow={1} flexDirection="column" justifyContent={verticalJustify(props.config.layout.statusVerticalAlign)}>
        <box width="100%" flexDirection="row" justifyContent={justifyContent(props.config.layout.statusContentAlign)}>
          <text style={textStyle(style(), style().muted)}>
            <For each={lineParts()}>
              {(part) =>
                part.key === "hint" ? (
                  <For each={hintSpans()}>
                    {(span) => (
                      <span style={textStyle(style(), span.role === "keys" ? style().accent : statusPartColor(props.config, "hint", watcherTone(), operationTone()))}>
                        {span.text}
                      </span>
                    )}
                  </For>
                ) : (
                  <span style={textStyle(style(), statusPartColor(props.config, part.key, watcherTone(), operationTone()))}>{part.text}</span>
                )
              }
            </For>
          </text>
        </box>
      </box>
    </box>
  );
}

type StatusTone = "muted" | "error" | "success" | "warning";
type StatusLineValues = Record<"hint" | "separator" | "watcher" | "operation", string>;

function fittedStatusLineValues(
  config: ResolvedTuiConfig,
  values: Omit<StatusLineValues, "hint">,
  width: number,
  mode: TuiStatusHintMode,
): StatusLineValues {
  const fitted: StatusLineValues = {
    ...values,
    operation: configuredMax(values.operation, config.layout.statusOperationMaxWidth, config),
    watcher: configuredMax(values.watcher, config.layout.statusWatcherMaxWidth, config),
    hint: "",
  };
  const used = templateWidth(config.labels.statusLineTemplate, fitted);
  let hintRoom = Math.max(0, width - used);
  if (config.layout.statusHintMaxWidth > 0) hintRoom = Math.min(hintRoom, config.layout.statusHintMaxWidth);
  fitted.hint = fittedStatusHint(config, hintRoom, mode);
  const shrinkOrder: Array<keyof StatusLineValues> = ["watcher", "operation"];
  for (const key of shrinkOrder) {
    const overflow = templateWidth(config.labels.statusLineTemplate, fitted) - width;
    if (overflow <= 0) break;
    if (fitted[key].length === 0) continue;
    fitted[key] = truncateText(fitted[key], Math.max(0, fitted[key].length - overflow), config.labels);
  }
  return fitted;
}

function statusPartColor(config: ResolvedTuiConfig, key: string | null, watcher: StatusTone, operation: StatusTone): string {
  const style = surface(config, "status");
  if (key === "watcher") return configuredToneColor(style, config.statusLineTones.watcher, toneColorFromStyle(style, watcher));
  if (key === "operation") return configuredToneColor(style, config.statusLineTones.operation, toneColorFromStyle(style, operation));
  if (key === "hint") return configuredToneColor(style, config.statusLineTones.hint, style.muted);
  if (key === "separator") return configuredToneColor(style, config.statusLineTones.separator, style.muted);
  return style.muted;
}

function toneColorFromStyle(style: TuiSurfaceStyle, tone: StatusTone): string {
  switch (tone) {
    case "error":
      return style.error;
    case "success":
      return style.success;
    case "warning":
      return style.warning;
    default:
      return style.fg;
  }
}
