// Session labels: the colours they come in, and which sessions carry which.

import type { Label, LabelBook } from "./api";

/** Label colours in the order the picker shows them, tuned to read on every theme. */
export const LABEL_COLORS: Record<string, string> = {
  red: "#f2716b",
  orange: "#f0a45d",
  yellow: "#e8c065",
  green: "#7fd17a",
  cyan: "#5fcfd6",
  blue: "#6aa8ff",
  purple: "#b58cf5",
  pink: "#f28cc0",
  gray: "#a3aab5",
};

export const LABEL_COLOR_NAMES = Object.keys(LABEL_COLORS);

/** The longest name a label can have (see termlab_core::labels). */
export const LABEL_NAME_MAX = 32;

export const NO_LABELS: LabelBook = { labels: [], sessions: {} };

/** A colour's shade; grey for one this version doesn't know. */
export function labelHex(color: string): string {
  return LABEL_COLORS[color] ?? LABEL_COLORS.gray;
}

/** Each labelled session's labels, in the labels' own order. */
export function labelsBySession(book: LabelBook): Map<string, Label[]> {
  const out = new Map<string, Label[]>();
  for (const [session, ids] of Object.entries(book.sessions)) {
    const labels = book.labels.filter((l) => ids.includes(l.id));
    if (labels.length > 0) out.set(session, labels);
  }
  return out;
}

/** The colour to offer for a new label: the first one no label uses yet. */
export function nextLabelColor(labels: Pick<Label, "color">[]): string {
  return LABEL_COLOR_NAMES.find((c) => !labels.some((l) => l.color === c)) ?? LABEL_COLOR_NAMES[labels.length % LABEL_COLOR_NAMES.length];
}

/** The book with one session's labels replaced, as saving them will leave it. */
export function withSessionLabels(book: LabelBook, session: string, ids: number[]): LabelBook {
  const sessions = { ...book.sessions };
  const kept = book.labels.map((l) => l.id).filter((id) => ids.includes(id));
  if (kept.length > 0) sessions[session] = kept;
  else delete sessions[session];
  return { ...book, sessions };
}
