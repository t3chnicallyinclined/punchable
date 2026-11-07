// The script's state on this phone (SPEC 2026-11-06 §12.4): one per topic, shared by the script page and the two
// sheets over it (Material, Claude on a line), so a line pulled or a version used lands on the page he is looking at.
// Every change writes the local draft (lib/scriptDraft: Documents on the phone) after a short settle, so nothing typed
// is lost offline or if the app dies; a save writes one version (lib/rant saveScript) and only when the words or the
// marks changed. Pure rules: rules/scriptRules.ts.
import { useSyncExternalStore } from 'react';
import { saveScript } from './rant';
import { dropDraft, writeDraft } from './scriptDraft';
import { changed, fromDoc, localDraft, toSave, type DocLine, type SLine } from '@/rules/scriptRules';

export type ScriptState = {
  topicId: string; topicText: string;
  /** the rant bit (null until the first save makes it) */
  rantId: string | null;
  /** the version the page was opened on or last saved (its unit), and its lines with their marks */
  unit: string | null; saved: DocLine[] | null; versions: number;
  lines: SLine[];
  /** the line he is on (Material puts a line after it) */
  at: number | null;
};

const states = new Map<string, ScriptState>();
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((f) => f());
const timers = new Map<string, ReturnType<typeof setTimeout>>();
const saving = new Map<string, Promise<Saved | null>>();
const DRAFT_SETTLE_MS = 400;

export type Saved = { id: string; unit: string; indexes: number[]; v: number; created: boolean };

function keepLocal(s: ScriptState) {
  clearTimeout(timers.get(s.topicId));
  timers.set(s.topicId, setTimeout(() => {
    const cur = states.get(s.topicId);
    if (!cur) return;
    const pending = changed(cur.lines, cur.saved);
    void (pending ? writeDraft(localDraft(cur.topicId, cur.rantId, cur.unit, cur.lines, true)) : dropDraft(cur.topicId));
  }, DRAFT_SETTLE_MS));
}

export const scripts = {
  get: (topic: string): ScriptState | null => states.get(topic) || null,
  /** Open (or re-open) a topic's script on these lines. */
  open(s: ScriptState) {
    states.set(s.topicId, s);
    emit();
  },
  /** Change the lines (and the line he is on); the local draft follows. */
  setLines(topic: string, lines: SLine[], at?: number | null) {
    const s = states.get(topic);
    if (!s) return;
    const n = { ...s, lines, ...(at !== undefined ? { at } : {}) };
    states.set(topic, n);
    emit();
    keepLocal(n);
  },
  setAt(topic: string, at: number | null) {
    const s = states.get(topic);
    if (!s || s.at === at) return;
    states.set(topic, { ...s, at });
    emit();
  },
  /** Forget a topic's page state (after an Undo of its version, or a delete): the next open reads the server. */
  forget(topic: string) {
    states.delete(topic);
    emit();
  },
  dirty: (topic: string): boolean => {
    const s = states.get(topic);
    return !!s && changed(s.lines, s.saved);
  },
  /** Write one version if the words or the marks changed; one save per topic at a time. Resolves to what was saved,
   *  or null (nothing to save). Throws when the version could not be written (the local draft keeps the words). */
  async save(topic: string): Promise<Saved | null> {
    const running = saving.get(topic);
    if (running) await running.catch(() => null);
    const s = states.get(topic);
    if (!s || !changed(s.lines, s.saved) || !toSave(s.lines).text) return null;
    const lines = s.lines;
    const p = (async () => {
      let made: string | null = null;
      try {
        const r = await saveScript({ topicId: s.topicId, topicText: s.topicText, rantId: s.rantId, lines, saved: s.saved, onBit: (id) => { made = id; } });
        if (!r) return null;
        const cur = states.get(topic) || s;
        const savedLines: DocLine[] = toSave(lines).lines.map((text, i) => ({ text, ...toSave(lines).marks[i] }));
        const versions = (r.created ? 0 : s.versions) + 1;
        states.set(topic, { ...cur, rantId: r.id, unit: r.unit, saved: savedLines, versions });
        emit();
        const after = states.get(topic) as ScriptState;
        void (changed(after.lines, after.saved)
          ? writeDraft(localDraft(topic, after.rantId, after.unit, after.lines, true))
          : dropDraft(topic));
        return { ...r, v: versions };
      } catch (e) {
        // the bit was made but its version failed: the next try writes onto it, never a second empty bit
        if (made) {
          const cur = states.get(topic);
          if (cur && !cur.rantId) states.set(topic, { ...cur, rantId: made });
        }
        const cur = states.get(topic) || s;
        await writeDraft(localDraft(topic, cur.rantId, cur.unit, cur.lines, true));
        throw e;
      }
    })();
    saving.set(topic, p);
    try {
      return await p;
    } finally {
      if (saving.get(topic) === p) saving.delete(topic);
    }
  },
};

/** One topic's script state, re-rendered on every change. */
export function useScript(topic: string): ScriptState | null {
  return useSyncExternalStore(
    (f) => { listeners.add(f); return () => { listeners.delete(f); }; },
    () => states.get(topic) || null,
    () => states.get(topic) || null,
  );
}

/** The page lines of a saved doc (a re-export so screens need one import). */
export const linesFrom = fromDoc;
