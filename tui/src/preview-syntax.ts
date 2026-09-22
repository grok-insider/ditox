import { SyntaxStyle } from "@opentui/core";
import type { TuiTheme } from "./theme";

export type PreviewBodyKind = "plain" | "markdown" | "code";

const heading = /^#{1,6} \S/m;
const fence = /^```[\w+-]*\s*$/m;
const list = /^\s*(?:[-*+]|\d+\.) \S/m;
const quote = /^\s*> \S/m;
const table = /^\s*\|.+\|\s*$/m;
const link = /\[[^\]]+\]\([^)\s]+\)/;
const codeStart = /^(?:#!|import |export |function |class |def |const |let |fn |pub |package )/m;

export function classifyPreview(content: string): PreviewBodyKind {
  const text = content.trim();
  if (!text) return "plain";
  if (fence.test(text) || heading.test(text) || list.test(text) || quote.test(text) || table.test(text) || link.test(text)) return "markdown";
  if (codeStart.test(text)) return "code";
  return "plain";
}

export function codeFiletype(content: string): string {
  const fenceLanguage = content.match(/^```([\w+-]+)/);
  if (fenceLanguage) return fenceLanguage[1];
  if (/^#!/.test(content) && /python/.test(content)) return "python";
  if (/^#!/.test(content)) return "bash";
  if (/\bfn\b|\bpub\b|\bimpl\b/.test(content)) return "rust";
  if (/\bfunction\b|\bconst\b|\blet\b|\bimport\b/.test(content)) return "javascript";
  return "text";
}

const syntaxStyles = new Map<string, SyntaxStyle>();

export function previewSyntaxStyle(theme: TuiTheme): SyntaxStyle {
  const key = [
    theme.name,
    theme.textPrimary,
    theme.textMuted,
    theme.accentCommand,
    theme.accentSuccess,
    theme.accentWarning,
    theme.accentText,
    theme.accentSearch,
  ].join("\u0000");
  const cached = syntaxStyles.get(key);
  if (cached) return cached;
  const style = SyntaxStyle.fromStyles({
    default: { fg: theme.textPrimary },
    comment: { fg: theme.textMuted, italic: true },
    keyword: { fg: theme.accentCommand },
    string: { fg: theme.accentSuccess },
    number: { fg: theme.accentWarning },
    function: { fg: theme.accentText },
    type: { fg: theme.accentSearch },
    heading: { fg: theme.accentText, bold: true },
    link: { fg: theme.accentCommand, underline: true },
  });
  syntaxStyles.set(key, style);
  return style;
}
