// The bit workbench's pure rules (no imports: node runs this file in tests/bitWork.test.mjs). lib/bitWork re-exports.

/** Which of several bits becomes the main when he says "make these one bit": the one with the most written
 *  versions, then the most nights on stage, then the first he picked. */
export function pickMain<T extends { id: string; versions?: number; sets?: number }>(items: T[]): T | null {
  if (!items.length) return null;
  let best = items[0];
  for (const it of items.slice(1)) {
    const a = [(it.versions || 0), (it.sets || 0)], b = [(best.versions || 0), (best.sets || 0)];
    if (a[0] > b[0] || (a[0] === b[0] && a[1] > b[1])) best = it;
  }
  return best;
}

/** The words of an answer worth keeping as a line: the answer with its [n] source marks removed, trimmed. */
export function answerLine(answer?: string | null): string {
  return (answer || '').replace(/\s*\[\d+\]/g, '').replace(/\s+/g, ' ').trim();
}

// A fresh bit (the "+" on Tonight and Bits, docs/design/SPEC-2026-10-30-create-flow.md §3): it exists before it has a
// name, under a marker title the first Save keys on; the first saved line then names it.
const FRESH_MARK = 'Untitled bit';

/** The placeholder title a new bit is created with: "Untitled bit YYYY-MM-DD HH:MM" (UTC minute; the bar shows "New bit"). */
export function freshBitTitle(now: Date): string {
  return `${FRESH_MARK} ${now.toISOString().slice(0, 16).replace('T', ' ')}`;
}

/** True only for a title still carrying the marker prefix (so "Untitled bits of wisdom" is a real title, not fresh). */
export function isFreshBit(title?: string | null): boolean {
  const t = (title || '').trim();
  return t === FRESH_MARK || t.startsWith(`${FRESH_MARK} `);
}

/** The name a fresh bit takes from what he wrote: the first seven words of the first non-empty line, trailing
 *  punctuation cut, at most 120 characters; "New bit" when there is nothing to name it from. */
export function nameFromFirstLine(text?: string | null): string {
  const line = (text || '').split(/\r?\n/).map((l) => l.trim()).find((l) => l) || '';
  const name = line.split(/\s+/).slice(0, 7).join(' ').replace(/[\s,.;:!?"'…]+$/, '').slice(0, 120).trim();
  return name || 'New bit';
}
