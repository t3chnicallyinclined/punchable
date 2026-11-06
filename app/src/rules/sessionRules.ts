// Film it's two sessions on a bit (docs/design/SPEC-2026-11-03-film-it.md §2.5, D10, D11; the old spec's S3 tracked
// changes and S7 rows): "Make it a rant" returns whole-line hunks over one version, he keeps or skips each one, and the
// kept ones are assembled into the new version's lines here; "Pick a hook" returns three openers and the one he uses
// arrives as one more hunk (a line inserted before line 1) through the same Keep → Save path. Pure: no React, no
// network; tests/sessionRules.test.mjs runs it under node (with a --red arm).

/** One tracked change, as the worker returns it (creative/rant.py): `line_no` is 1-based over the version's lines;
 *  an `add` goes after `line_no` (0 = before the first). `was` is his line, checked word for word on the server. */
export type Hunk = { op: 'change' | 'cut' | 'add'; line_no: number; was: string; now: string; why?: string | null };
export type Opener = { text: string; why?: string | null };
/** Keep / Skip per hunk: undefined = not decided yet (reads as not kept). */
export type Marks = (boolean | undefined)[];

const isInt = (x: unknown): x is number => typeof x === 'number' && Number.isInteger(x);

/** A hunk that fits these lines: one this phone can apply. Anything else is left out (the server already checked). */
export function fits(h: Hunk, n: number): boolean {
  if (!h || !isInt(h.line_no)) return false;
  if (h.op === 'add') return h.line_no >= 0 && h.line_no <= n && !!(h.now || '').trim();
  if (h.op === 'cut') return h.line_no >= 1 && h.line_no <= n;
  if (h.op === 'change') return h.line_no >= 1 && h.line_no <= n && !!(h.now || '').trim();
  return false;
}

/** The new version's lines: the base with every KEPT hunk applied (a skipped or undecided hunk changes nothing). Adds
 *  after the same line keep the order Claude gave them; a line changed and cut (never sent) takes the first kept. */
export function applyHunks(base: string[], hunks: Hunk[], kept: Marks): string[] {
  const n = base.length;
  const line = new Map<number, string | null>();          // line_no → new text, or null for a cut
  const adds = new Map<number, string[]>();               // after line_no → added lines
  hunks.forEach((h, i) => {
    if (!kept[i] || !fits(h, n)) return;
    if (h.op === 'add') adds.set(h.line_no, [...(adds.get(h.line_no) || []), h.now.trim()]);
    else if (!line.has(h.line_no)) line.set(h.line_no, h.op === 'cut' ? null : h.now.trim());
  });
  const out = [...(adds.get(0) || [])];
  for (let k = 1; k <= n; k++) {
    const v = line.has(k) ? line.get(k) : base[k - 1];
    if (v != null) out.push(v);
    out.push(...(adds.get(k) || []));
  }
  return out;
}

/** Save is rendered only once a change is kept (3c §3.6), and only when the result still has a line. */
export function canSave(base: string[], hunks: Hunk[], kept: Marks): boolean {
  return kept.some(Boolean) && applyHunks(base, hunks, kept).length > 0;
}

/** VoiceOver for one change: "Change 2 of 4. Removed: …. Added: …" (strikethrough is not spoken, so the label says it). */
export function hunkLabel(h: Hunk, i: number, count: number): string {
  const head = `Change ${i + 1} of ${count}.`;
  if (h.op === 'add') return `${head} Added${h.line_no === 0 ? ' at the start' : ` after line ${h.line_no}`}: ${h.now}`;
  if (h.op === 'cut') return `${head} Removed: ${h.was}`;
  return `${head} Removed: ${h.was}. Added: ${h.now}`;
}

/** The opener he uses, as the one hunk the Keep → Save path takes: a new line before line 1. */
export const hookHunk = (text: string): Hunk => ({ op: 'add', line_no: 0, was: '', now: text.trim() });

/** "Show others" asks for three more once (Film it §2.5): rounds = how many sets of openers he has seen. */
export const canShowOthers = (rounds: number) => rounds < 2;

/** The snack after a save: "Saved as v9 · rant" / "Saved as v9". */
export const savedWord = (n: number, cut: 'stage' | 'rant') => `Saved as v${n}${cut === 'rant' ? ' · rant' : ''}`;

/** Where a session stands, from the enqueue answer or a poll of the job: a result, still waiting, or failed. */
export type JobView<T> = { state: 'done'; result: T } | { state: 'waiting'; job: string | null; waitingFor?: string | null } | { state: 'failed'; error: string };
export function jobView<T>(r: any): JobView<T> {
  if (!r || typeof r !== 'object') return { state: 'failed', error: 'no answer' };
  // the enqueue's 202, or a poll of a job not finished yet
  if (r.status === 'queued' || r.status === 'claimed') return { state: 'waiting', job: r.job?.id ?? r.id ?? null, waitingFor: r.waiting_for ?? null };
  if (r.status === 'failed') return { state: 'failed', error: String(r.error || 'failed') };
  // a poll of a finished job carries its result; the enqueue's 200 (cached) IS the result
  if (r.status === 'done' && r.result && typeof r.result === 'object') return { state: 'done', result: r.result as T };
  if (r.status === 'done') return { state: 'failed', error: 'no result' };
  return { state: 'done', result: r as T };
}

/** The one line a finished session shows when there is nothing to keep (the old spec's "nothing to suggest"). */
export function emptyWord(mode: 'rant' | 'hook', status?: string | null, why?: string | null): string | null {
  if (status === 'ok') return null;
  if (status === 'provider_error') return why ? `Claude didn’t answer: ${why}` : 'Claude didn’t answer.';
  return mode === 'rant' ? 'Claude has nothing to change here.' : 'Claude has no openers for this one.';
}
