import type { IBuffer, IBufferCell, ILink, ILinkProvider, Terminal } from "@xterm/xterm";

/** A run of underlined cells on one buffer row: `start` inclusive, `end` exclusive, 0-based. */
interface Run {
  y: number;
  start: number;
  end: number;
  text: string;
}

/** The longest label worth looking up, in rows: a link label rarely wraps more than this. */
const MAX_ROWS = 4;

/**
 * Links for underlined text whose target isn't on the screen. Codex prints a Markdown link
 * (`[label](/path/report.html)`) as just its underlined label, sending an OSC 8 hyperlink only
 * for web links, so `open(label)` looks the target up elsewhere (the session's transcript).
 * Register it after the other link providers: xterm prefers earlier ones, so printed URLs and
 * real hyperlinks keep their own handling.
 */
export function underlinedLinkProvider(
  term: Terminal,
  open: (event: MouseEvent, label: string) => void,
): ILinkProvider {
  return {
    provideLinks(lineNumber, callback) {
      const buf = term.buffer.active;
      const cell = buf.getNullCell();
      const y = lineNumber - 1;
      const links: ILink[] = runsOn(buf, y, cell).map((run, i, all) => {
        const parts = [run];
        // A label Codex wrapped: underline running to the end of one row and on from the
        // start of the next, with nothing else in between.
        if (i === 0) {
          for (let p = run; parts.length < MAX_ROWS && blankBefore(buf, p.y, p.start, cell); ) {
            const prev = runsOn(buf, p.y - 1, cell).at(-1);
            if (!prev || !blankFrom(buf, prev.y, prev.end, cell)) break;
            parts.unshift(prev);
            p = prev;
          }
        }
        if (i === all.length - 1) {
          for (let p = run; parts.length < MAX_ROWS && blankFrom(buf, p.y, p.end, cell); ) {
            const next = runsOn(buf, p.y + 1, cell)[0];
            if (!next || !blankBefore(buf, next.y, next.start, cell)) break;
            parts.push(next);
            p = next;
          }
        }
        const first = parts[0];
        const last = parts[parts.length - 1];
        // A row the terminal wrapped itself continues mid-word; one the agent wrapped, after a space.
        const label = parts.map((p, k) => (k > 0 && !buf.getLine(p.y)?.isWrapped ? " " : "") + p.text).join("");
        return {
          range: { start: { x: first.start + 1, y: first.y + 1 }, end: { x: last.end, y: last.y + 1 } },
          text: label,
          decorations: { pointerCursor: true, underline: false },
          activate: (event: MouseEvent) => open(event, label),
        };
      });
      callback(links.length ? links : undefined);
    },
  };
}

/** The underlined runs on row `y`, trimmed; runs that are themselves URLs are left out. */
function runsOn(buf: IBuffer, y: number, cell: IBufferCell): Run[] {
  const line = buf.getLine(y);
  if (!line) return [];
  const runs: Run[] = [];
  let cur: { start: number; text: string } | null = null;
  const close = (end: number) => {
    if (!cur) return;
    const lead = cur.text.length - cur.text.trimStart().length;
    const text = cur.text.trim();
    // Columns and characters line up only for single-width text; trimming spaces keeps that.
    if (text && !/^[a-z][\w+.-]*:\/\//i.test(text)) {
      runs.push({ y, start: cur.start + lead, end: end - (cur.text.length - lead - text.length), text });
    }
    cur = null;
  };
  for (let x = 0; x < line.length; x++) {
    if (!line.getCell(x, cell)) break;
    if (cell.getWidth() === 0) continue; // the right half of a wide character
    if (cell.isUnderline()) {
      cur ??= { start: x, text: "" };
      cur.text += cell.getChars() || " ";
    } else {
      close(x);
    }
  }
  close(line.length);
  return runs;
}

function blankBefore(buf: IBuffer, y: number, x: number, cell: IBufferCell): boolean {
  const line = buf.getLine(y);
  if (!line) return false;
  for (let i = 0; i < x; i++) if (line.getCell(i, cell)?.getChars().trim()) return false;
  return true;
}

function blankFrom(buf: IBuffer, y: number, x: number, cell: IBufferCell): boolean {
  const line = buf.getLine(y);
  if (!line) return false;
  for (let i = x; i < line.length; i++) if (line.getCell(i, cell)?.getChars().trim()) return false;
  return true;
}
