// Rant scripts saved only on this phone (offline, or the app died mid-line): sent as a version of their rant once he
// is signed in and online. Moved out of the Rant tab when the tabs became Bits · Sets · Tapes (SPEC 2026-11-16 §3.6,
// Slice 6: "flushed as a version at launch by the code that does it today, moved to the root layout, before the Rant
// screens are removed"); the draft file is dropped only after the save answers. A topic's page saves its own.
import { get } from './api';
import { saveScript } from './rant';
import { dropDraft, pendingDrafts } from './scriptDraft';
import { scripts } from './scriptStore';
import { fromDoc } from '@/rules/scriptRules';
import type { Topic } from '@/rules/rantRules';

let flushing = false;
/** Scripts saved only on this phone (offline, or the app died): sent when the tab shows, over the version they were
 *  written on; a draft of an older version than the server's is the server's loss to win (it is dropped). */
async function flushWith(topics: Topic[]) {
  if (flushing) return;
  flushing = true;
  try {
    for (const d of await pendingDrafts()) {
      if (scripts.get(d.topic)) continue; // its page is open: that page saves it
      const t = topics.find((x) => x.id === d.topic);
      if (!t) continue;
      const serverUnit = t.unit || null;
      // a draft of an older version than the server's loses (a newer save wins); a first rant or "Start another rant"
      // (no rant yet on the draft) is a new bit
      if (d.rant && (t.rants || [t.rant_id]).every((r) => r !== d.rant)) { await dropDraft(d.topic); continue; }
      if (d.rant && d.rant === t.rant_id && serverUnit && serverUnit !== d.base) { await dropDraft(d.topic); continue; }
      try {
        await saveScript({ topicId: t.id, topicText: t.text, rantId: d.rant || null, lines: fromDoc(d.lines), saved: null });
        await dropDraft(d.topic);
      } catch { break; } // still offline: the rest wait
    }
  } finally {
    flushing = false;
  }
}

/** From the root layout: only when a draft is waiting does it read the topics (one GET), then saves each draft. */
export async function flushScriptDrafts(): Promise<void> {
  if (flushing) return;
  const waiting = await pendingDrafts().catch(() => []);
  if (!waiting.length) return;
  const r = await get<{ topics: Topic[] }>('/api/topics').catch(() => null);
  if (r?.topics) await flushWith(r.topics);
}
