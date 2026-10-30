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
