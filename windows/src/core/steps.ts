// Ticker steps that are more than text: a file edit is stored as a marker plus
// the edit's id, and drawn with its +N −M by whoever shows it.

import type { AgentTask, EditInfo } from "./state";

/** Private-use character: never typed by an agent, so never mistaken for one. */
export const EDIT_MARK = "\uE001";

export function editStep(id: string): string {
  return EDIT_MARK + id;
}

export function editOf(task: AgentTask | null | undefined, step: string | undefined): EditInfo | null {
  if (!task || !step?.startsWith(EDIT_MARK)) return null;
  return task.edits?.find((e) => e.id === step.slice(EDIT_MARK.length)) ?? null;
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** The step as plain text, for places that only show text. */
export function stepPlain(task: AgentTask | null | undefined, step: string | undefined): string {
  if (step == null) return "";
  const edit = editOf(task, step);
  if (edit) return `${baseName(edit.path)} +${edit.added} −${edit.removed}`;
  return step.startsWith(EDIT_MARK) ? "Edit" : step;
}

/**
 * An agent's last message as one line: its first real paragraph, without the
 * Markdown, whitespace collapsed.
 */
export function oneLine(text: string, max = 200): string {
  const body = text.replace(/<think>[\s\S]*?(<\/think>|$)/g, "").replace(/```[\s\S]*?(```|$)/g, " ");
  const para = body.split(/\n\s*\n/).map((p) => p.trim()).find((p) => p.length > 0) ?? "";
  const plain = para
    .split("\n")
    .map((l) => l.replace(/^\s{0,3}(#{1,6}\s+|>\s?|[-*+]\s+|\d{1,3}[.)]\s+)/, ""))
    .join(" ")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/(\*\*|__|`)/g, "")
    .replace(/(^|\s)[*_](\S[^*_]*?)[*_](?=\s|$|[.,;:!?])/g, "$1$2")
    .replace(/\s+/g, " ")
    .trim();
  return plain.length > max ? `${plain.slice(0, max - 1).trimEnd()}…` : plain;
}
