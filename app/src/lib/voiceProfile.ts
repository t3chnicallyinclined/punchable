// The voice profile screen's rules (Settings > Voice profile), pure so tests/voiceProfile.test.mjs can prove them:
// what the API holds (GET/PUT /api/profile/voice, api-dev/profile_api.py), when Save is on, what Undo restores,
// and the status line. The text itself is his; nothing here sends it anywhere.

export type VoiceVersion = { text: string; updated_at: string | null; by: string | null };
/** One line the app learned from him (creative/learned.py): the words, the bit it went to, the day. */
export type LearnedLine = { text: string; bit: string | null; date: string | null };
/** The learned section of the document: maintained by the server from his keeps, never typed, never editable here. */
export type Learned = { kept: LearnedLine[]; dropped: LearnedLine[]; updated_at: string | null };
export type VoiceDoc = {
  kind: 'voice'; text: string | null; updated_at: string | null; by: string | null; chars: number; previous: VoiceVersion | null;
  learned?: Learned | null;
};
export const MAX_CHARS = 60000; // api-dev/profile_api.py MAX_CHARS

/** What the API keeps: \r\n -> \n, no leading/trailing blank lines (so a saved text compares equal to its draft). */
export function normalize(t: string | null | undefined): string {
  return (t || '').replace(/\r\n/g, '\n').replace(/^\n+/, '').replace(/\n+$/, '');
}

export function wordCount(t: string | null | undefined): number {
  const n = normalize(t);
  return n ? n.split(/\s+/).filter(Boolean).length : 0;
}

/** Save is on only when there is something new to save, and it fits. `why` is the plain reason it is off. */
export function canSave(draft: string, saved: string | null): { ok: boolean; why: string | null } {
  const d = normalize(draft);
  if (!d.trim()) return { ok: false, why: 'Write something first.' };
  if (d.length > MAX_CHARS) return { ok: false, why: `Too long by ${(d.length - MAX_CHARS).toLocaleString('en-US')} characters.` };
  if (d === normalize(saved)) return { ok: false, why: 'Nothing changed.' };
  return { ok: true, why: null };
}

/** The version Undo restores after a save: the document's previous version (the API keeps exactly one). */
export function undoVersion(doc: VoiceDoc | null | undefined): VoiceVersion | null {
  const p = doc?.previous;
  return p && typeof p.text === 'string' && normalize(p.text).trim() ? p : null;
}

function dateWords(iso: string | null | undefined): string {
  if (!iso) return '';
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return '';
  return new Date(t).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

function byWords(by: string | null | undefined): string {
  if (!by) return '';
  if (by.startsWith('import:')) return 'loaded from your desk';
  if (by.startsWith('ai:')) return 'written by the AI';
  return 'by you';
}

/** "Not written yet. The AI drafts without it." / "Saved Oct 28, 2026 · 1,240 words · by you". */
export function statusLine(doc: VoiceDoc | null | undefined): string {
  if (!doc || !doc.text) return 'Not written yet. The AI drafts without it.';
  const parts = [`Saved ${dateWords(doc.updated_at)}`.trim(), `${wordCount(doc.text).toLocaleString('en-US')} words`];
  const by = byWords(doc.by);
  if (by) parts.push(by);
  return parts.join(' · ');
}

/** The snackbar after a save; the Undo on it restores `undoVersion(doc)` when there is one. */
export function savedWords(doc: VoiceDoc): string {
  return `Saved · ${wordCount(doc.text).toLocaleString('en-US')} words`;
}

/** The "What it has learned" line: counts, or the honest empty line. The server fills this from his keeps
 *  ("Use as mine" on a riff or punch-up draft, api-dev/profile_api.py learn_kept); nothing here is typed. */
export function learnedWords(l: Learned | null | undefined): string {
  const k = l?.kept?.length || 0;
  const d = l?.dropped?.length || 0;
  if (!k && !d) return 'Nothing yet. It learns when you take one of its drafts as yours.';
  const when = dateWords(l?.updated_at);
  return `${k} line${k === 1 ? '' : 's'} kept · ${d} passed over${when ? ` · last ${when}` : ''}`;
}

/** The learned lines as shown, newest first (the server already orders them), each with its month. */
export function learnedRows(lines: LearnedLine[] | null | undefined, cap = 25): { text: string; when: string }[] {
  return (lines || []).slice(0, cap).map((x) => ({ text: x.text, when: dateWords(x.date) }));
}
