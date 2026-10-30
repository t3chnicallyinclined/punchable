// The one place a time or a date becomes words (docs/ARCHITECTURE.md §7.1). Before 2026-10-29 four formatters and six
// month arrays did this in theme.ts, laughs.ts, searchRows.ts, simple.ts, ask.tsx, today.tsx and set/[id].tsx;
// tests/format.test.mjs pins every old caller's exact output. Pure: no React, no React Native, runs under node.

export const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
export const MONTHS_LONG = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

const isNum = (x: unknown): x is number => typeof x === 'number' && isFinite(x);

/**
 * Seconds as m:ss ("1:43"; 61 minutes read "61:01"). Rounds to the nearest second (a length); `floor` for a running
 * clock, which must not show a second before it has passed. Not a number → "–" (an unknown length).
 */
export function mmss(s: number | null | undefined, opts?: { floor?: boolean }): string {
  if (!isNum(s)) return '–';
  const t = Math.max(0, opts?.floor ? Math.floor(s) : Math.round(s));
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`;
}

/** Milliseconds as m:ss for the record clock and the lengths it wrote: whole seconds passed (floor), never "–"
 *  (a clock reads "0:00" before it starts). The old theme.ts fmtMs. */
export const clockMs = (ms: number | null | undefined): string => mmss((ms || 0) / 1000, { floor: true });

/** "Mar 2025" from an ISO date; just the year when that is all there is. */
export function monthYear(d?: string | null): string {
  if (!d) return '';
  const m = Number(d.slice(5, 7));
  return m >= 1 && m <= 12 ? `${MONTHS[m - 1]} ${d.slice(0, 4)}` : d.slice(0, 4);
}

/** "Mar 4, 2025" from an ISO date (a date-time's date part); '' when there is none. */
export function fmtDate(iso?: string | null): string {
  if (!iso) return '';
  const [y, m, d] = iso.slice(0, 10).split('-').map(Number);
  return `${MONTHS[(m || 1) - 1]} ${d}, ${y}`;
}

/** "Mar 4 '25": the short show date on a row; '' when there is none. */
export function fmtShortDate(iso?: string | null): string {
  if (!iso) return '';
  const [y, m, d] = iso.slice(0, 10).split('-').map(Number);
  return `${MONTHS[(m || 1) - 1]} ${d} '${String(y).slice(2)}`;
}
